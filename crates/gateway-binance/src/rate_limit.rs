//! Глобальный token-bucket rate limiter для REST-вызовов.
//!
//! Binance считает rate limit "weight per minute" per IP. У Spot и Futures
//! разные лимиты (Spot: 6000/min, Futures: 2400/min) и разные хосты
//! (api.binance.com vs fapi.binance.com), поэтому их лимитеры независимы.
//!
//! Здесь мы держим ~50% бюджета как безопасный target:
//! * Spot: 33 weight/sec (≈2000/min из 6000)
//! * Futures: 30 weight/sec (≈1800/min из 2400)
//!
//! Используется в первую очередь для REST snapshot orderbook'а
//! (limit=1000 = 20 weight за вызов).

use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::trace;

/// Token bucket с непрерывным refill'ом.
pub struct WeightLimiter {
    /// Скорость пополнения токенов (единиц weight в секунду).
    rate_per_sec: f64,
    /// Максимальное число токенов (= burst capacity).
    capacity: f64,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

impl WeightLimiter {
    pub fn new(rate_per_sec: f64, capacity: f64) -> Self {
        Self {
            rate_per_sec,
            capacity,
            state: Mutex::new(BucketState {
                tokens: capacity,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Дожидается пока в bucket'е будет хотя бы `weight` токенов, потом
    /// списывает их. Если запрошенный weight больше capacity — берём
    /// max доступного и возвращаемся к caller'у (т.е. ничего страшного
    /// не произойдёт но wait'ы могут быть длинными).
    pub async fn acquire(&self, weight: u32) {
        let want = weight as f64;
        loop {
            let wait = {
                let mut state = self.state.lock().await;
                let now = Instant::now();
                let elapsed = now.saturating_duration_since(state.last_refill).as_secs_f64();
                state.tokens = (state.tokens + elapsed * self.rate_per_sec).min(self.capacity);
                state.last_refill = now;

                if state.tokens >= want {
                    state.tokens -= want;
                    trace!(
                        weight,
                        remaining = state.tokens,
                        rate = self.rate_per_sec,
                        "weight acquired"
                    );
                    return;
                }

                // Нужно подождать, пока накопится достаточно.
                let deficit = want - state.tokens;
                Duration::from_secs_f64(deficit / self.rate_per_sec)
            };
            // Спим за пределами lock'а.
            tokio::time::sleep(wait + Duration::from_millis(5)).await;
        }
    }
}

/// Глобальные лимитеры. Создаются при первом обращении.
/// Spot: 33 weight/sec (50% от 6000/min).
/// Futures: 30 weight/sec (75% от 2400/min — Futures cheaper budget,
/// но 50% = 20/sec слишком медленно для initial bootstrap'а 600 символов).
pub fn spot_limiter() -> &'static WeightLimiter {
    static L: OnceLock<WeightLimiter> = OnceLock::new();
    L.get_or_init(|| WeightLimiter::new(33.0, 60.0))
}

pub fn futures_limiter() -> &'static WeightLimiter {
    static L: OnceLock<WeightLimiter> = OnceLock::new();
    L.get_or_init(|| WeightLimiter::new(30.0, 60.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_within_capacity_is_instant() {
        let l = WeightLimiter::new(10.0, 50.0);
        let start = Instant::now();
        l.acquire(20).await;
        l.acquire(20).await;
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50), "elapsed: {:?}", elapsed);
    }

    #[tokio::test]
    async fn acquire_above_capacity_throttles() {
        let l = WeightLimiter::new(20.0, 20.0);
        l.acquire(20).await; // выпьем capacity
        let start = Instant::now();
        l.acquire(10).await; // должен подождать ~500ms
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(400),
            "elapsed too fast: {:?}",
            elapsed
        );
    }
}
