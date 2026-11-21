use crate::types::*;
use std::pin::Pin;
use futures::Stream;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    OrderBook(OrderBook),
    Trade(Trade),
    Candle(Candle),
    Ticker(Ticker),
    FundingRate(FundingRate),
    MarkPrice(MarkPrice),
    Liquidation(Liquidation),
    Info(String),
}

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

pub struct Subscription {
    _cancel: tokio::sync::oneshot::Sender<()>,
}

impl Subscription {
    pub fn new(cancel: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { _cancel: cancel }
    }
}

/// Следующее сообщение `rx`, пока потребитель `out` жив.
///
/// Таски-пересыльщики (`raw → out`) ждут `rx.recv()` вечно, если биржа
/// молчит, и держат сырой канал открытым — WS-таск не узнаёт, что стрим
/// бросили, и крутит соединение с реконнектами до рестарта процесса.
/// `None` — и когда `rx` закрыт, и когда потребитель `out` ушёл: в обоих
/// случаях пересыльщику пора выходить.
pub async fn recv_while_open<T, U>(
    rx: &mut tokio::sync::mpsc::Receiver<T>,
    out: &tokio::sync::mpsc::Sender<U>,
) -> Option<T> {
    let closed = out.closed();
    let recv = rx.recv();
    futures::pin_mut!(closed, recv);
    // `select` опрашивает левую ветку первой — закрытый выход важнее
    // готового сообщения.
    match futures::future::select(closed, recv).await {
        futures::future::Either::Left(_) => None,
        futures::future::Either::Right((msg, _)) => msg,
    }
}

/// Сколько WS-соединение может молчать, прежде чем его сочтут мёртвым.
///
/// Полуоткрытый TCP (сеть пропала без FIN/RST) не даёт ни ошибки чтения,
/// ни конца стрима: `read.next()` ждёт вечно, а запись ping уходит в буфер
/// ядра без ошибки ещё ~15 минут ретрансмитов. Книга при этом замерзает,
/// и downstream держит снятые стенки живыми. Каждый цикл чтения шлёт свой
/// ping не реже раза в 30 с, и биржа на него отвечает, так что на живом
/// соединении кадр приходит минимум дважды за это окно.
pub const WS_READ_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Сколько ждать TCP + TLS + WS-upgrade при (пере)подключении.
///
/// Без таймаута застрявший handshake (SYN без ответа, TLS посреди обрыва)
/// держит цикл в `connect_async` вечно: ни реконнекта, ни backoff, книга
/// замёрзла. С таймаутом это обычная ошибка подключения.
pub const WS_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Подключение с [`WS_CONNECT_TIMEOUT`]: по таймауту — ошибка
/// `io::ErrorKind::TimedOut` того же типа, что у самого подключения
/// (у `tungstenite::Error` есть `From<io::Error>`), так что существующие
/// ветки ошибок и backoff обрабатывают его без изменений.
pub async fn connect_timeout<F, T, E>(connect: F) -> std::result::Result<T, E>
where
    F: std::future::Future<Output = std::result::Result<T, E>>,
    E: From<std::io::Error>,
{
    match tokio::time::timeout(WS_CONNECT_TIMEOUT, connect).await {
        Ok(result) => result,
        Err(_) => Err(E::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "ws connect timed out",
        ))),
    }
}

/// Дедлайн тишины чтения WS-соединения.
///
/// Любой принятый кадр сдвигает дедлайн ([`ReadIdle::touch`]); ветка
/// `idle.expired()` в `select!` срабатывает, когда кадров не было дольше
/// таймаута, — цикл выходит в обычный путь реконнекта (resubscribe →
/// snapshot → diff). В `select!` ставить с `biased;` после ветки чтения:
/// иначе после долгой блокировки обработчика (медленный downstream)
/// случайный выбор может взять истёкший дедлайн при готовом кадре в буфере.
pub struct ReadIdle {
    timeout: std::time::Duration,
    deadline: tokio::time::Instant,
}

impl ReadIdle {
    pub fn new(timeout: std::time::Duration) -> Self {
        Self {
            timeout,
            deadline: tokio::time::Instant::now() + timeout,
        }
    }

    /// Пришёл кадр (или поднято новое соединение) — отсчёт заново.
    pub fn touch(&mut self) {
        self.deadline = tokio::time::Instant::now() + self.timeout;
    }

    /// Будущее, которое завершится на дедлайне. Пересоздаётся на каждом
    /// витке `select!`, дедлайн двигает только [`ReadIdle::touch`].
    pub fn expired(&self) -> tokio::time::Sleep {
        tokio::time::sleep_until(self.deadline)
    }

    pub fn timeout(&self) -> std::time::Duration {
        self.timeout
    }
}

/// То же, что [`recv_while_open`], для произвольного стрима.
pub async fn next_while_open<S, U>(
    stream: &mut S,
    out: &tokio::sync::mpsc::Sender<U>,
) -> Option<S::Item>
where
    S: Stream + Unpin,
{
    use futures::StreamExt;
    let closed = out.closed();
    let next = stream.next();
    futures::pin_mut!(closed, next);
    match futures::future::select(closed, next).await {
        futures::future::Either::Left(_) => None,
        futures::future::Either::Right((item, _)) => item,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn forwarder_exits_when_consumer_drops_on_silent_source() {
        let (_raw_tx, mut raw_rx) = mpsc::channel::<u32>(4);
        let (out_tx, out_rx) = mpsc::channel::<u32>(4);
        drop(out_rx);
        // Источник молчит, но выход закрыт — пересыльщик не должен висеть.
        assert_eq!(recv_while_open(&mut raw_rx, &out_tx).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn read_idle_fires_only_after_silence() {
        let timeout = std::time::Duration::from_secs(60);
        let mut idle = ReadIdle::new(timeout);
        tokio::time::advance(std::time::Duration::from_secs(50)).await;
        // Кадр на 50-й секунде — отсчёт заново, через 59 с дедлайн не наступил.
        idle.touch();
        let early = tokio::time::timeout(std::time::Duration::from_secs(59), idle.expired()).await;
        assert!(early.is_err(), "дедлайн сработал раньше таймаута после touch");
        // Ещё секунда тишины — дедлайн наступил.
        let late = tokio::time::timeout(std::time::Duration::from_secs(2), idle.expired()).await;
        assert!(late.is_ok(), "дедлайн не сработал после таймаута тишины");
    }

    #[tokio::test(start_paused = true)]
    async fn connect_timeout_turns_stalled_handshake_into_error() {
        let stalled = std::future::pending::<std::result::Result<(), std::io::Error>>();
        let err = connect_timeout(stalled)
            .await
            .expect_err("застрявший handshake не превратился в ошибку");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        let ready = connect_timeout(async { Ok::<u8, std::io::Error>(7) }).await;
        assert_eq!(ready.unwrap(), 7);
    }

    #[tokio::test]
    async fn forwarder_passes_messages_while_consumer_alive() {
        let (raw_tx, mut raw_rx) = mpsc::channel::<u32>(4);
        let (out_tx, _out_rx) = mpsc::channel::<u32>(4);
        raw_tx.send(7).await.unwrap();
        assert_eq!(recv_while_open(&mut raw_rx, &out_tx).await, Some(7));
        let mut s = futures::stream::iter([1u32]);
        assert_eq!(next_while_open(&mut s, &out_tx).await, Some(1));
    }
}
