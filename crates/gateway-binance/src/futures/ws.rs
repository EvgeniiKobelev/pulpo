use crate::futures::mapper::*;
use crate::futures::rest::BinanceFuturesRest;
use crate::rate_limit::futures_limiter;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use rust_decimal::Decimal;
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

/// См. spot/ws.rs — отдаём 1000 уровней (всю поддерживаемую глубину
/// LocalOrderBook'а), чтобы покрыть ±2% от mid на тонко-тиковых coins'ах.
const TOP_LEVELS: usize = 1000;
// 4096→16384 (C): дольше держим bridging-буфер между snapshot и live при
// re-sync, не проваливая bootstrap под burst'ом reconnect-ов.
const MAX_BUFFER: usize = 16384;
/// После стольких подряд неудачных bootstrap'ов считаем книгу безнадёжно
/// протухшей и эмитим purge (qty=0 по ранее отданным уровням), чтобы
/// downstream снял зомби, вместо того чтобы показывать их до следующего
/// успешного re-sync (на горячих символах — минуты).
const PURGE_AFTER_FAILURES: u32 = 3;
/// `/fapi/v1/depth?limit=1000` weight.
const DEPTH_WEIGHT: u32 = 20;

const WS_URL: &str = "wss://fstream.binance.com/ws";
const COMBINED_WS_URL: &str = "wss://fstream.binance.com/stream";

/// Maximum number of streams per single WebSocket connection.
/// Binance allows up to 200 but we stay well below the limit
/// to avoid connection resets caused by excessive data throughput.
const MAX_STREAMS_PER_CONNECTION: usize = 100;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to a Binance Futures WebSocket endpoint, optionally send a SUBSCRIBE
/// message, and return a [`BoxStream`] that yields parsed JSON values.
///
/// If `streams` is non-empty a SUBSCRIBE frame is sent after connecting.
/// For combined-stream URLs the subscription is implicit in the URL query string,
/// so pass an empty `Vec`.
///
/// The connection is automatically re-established with exponential back-off
/// whenever the remote side disconnects.
async fn subscribe_and_stream(
    url: &str,
    streams: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let url = url.to_string();

    let (ws_stream, _) =
        connect_async(&url)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            })?;

    let (mut write, read) = ws_stream.split();

    // Send SUBSCRIBE message when using the single-stream endpoint.
    if !streams.is_empty() {
        let sub = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": streams.clone(),
            "id": 1
        });
        write
            .send(Message::text(sub.to_string()))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            })?;
    }

    // 8192→32768 (C): больше запас Layer-1 relief-клапана до дропа depth-фрейма
    // при стале downstream. Дроп само-лечится через re-sync, но реже = меньше
    // транзиторных зомби.
    let (tx, rx) = mpsc::channel::<serde_json::Value>(32768);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);
        let mut dropped: u64 = 0;
        let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
        ping_interval.tick().await; // skip first immediate tick

        'outer: loop {
            // ---- message read loop with periodic ping ----
            loop {
                tokio::select! {
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("result").is_some() && json.get("id").is_some() {
                                        continue;
                                    }
                                    // Layer 1: never block the read/ping loop on a slow
                                    // downstream. Blocking here stalls pong replies →
                                    // binance RSTs the connection → every symbol on it
                                    // sequence-gaps at once → re-sync storm → stale-book
                                    // zombies. Drop on full instead: pulpo's own sequence
                                    // validation turns a dropped diff into a clean
                                    // per-symbol re-sync (self-healing), no reset.
                                    match tx.try_send(json) {
                                        Ok(()) => {}
                                        Err(mpsc::error::TrySendError::Full(_)) => {
                                            dropped += 1;
                                            if dropped % 1000 == 1 {
                                                warn!(
                                                    dropped,
                                                    "Binance Futures WS: downstream full, dropping depth frame (will re-sync)"
                                                );
                                            }
                                        }
                                        Err(mpsc::error::TrySendError::Closed(_)) => {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                if write.send(Message::Pong(data)).await.is_err() {
                                    warn!("Binance Futures WS pong send failed");
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Binance Futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Binance Futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Binance Futures WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = ping_interval.tick() => {
                        if write.send(Message::Ping(vec![].into())).await.is_err() {
                            warn!("Binance Futures WS ping send failed");
                            break;
                        }
                    }
                }
            }

            // ---- reconnect with exponential back-off ----
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Binance Futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(&url).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        if !streams.is_empty() {
                            let sub = serde_json::json!({
                                "method": "SUBSCRIBE",
                                "params": streams.clone(),
                                "id": 1
                            });
                            if new_write
                                .send(Message::text(sub.to_string()))
                                .await
                                .is_err()
                            {
                                warn!("Binance Futures WS subscribe failed after reconnect");
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                continue;
                            }
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        ping_interval.reset();
                        info!("Binance Futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Binance Futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Binance Futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream incremental order-book depth updates for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let stream_name = format!("{}@depth@100ms", unified_to_binance(symbol).to_lowercase());
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceFuturesWsDepthRaw = serde_json::from_value(json).ok()?;
        Some(raw.into_orderbook())
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let stream_name = format!("{}@trade", unified_to_binance(symbol).to_lowercase());
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceFuturesWsTradeRaw = serde_json::from_value(json).ok()?;
        Some(raw.into_trade())
    })))
}

/// Stream kline/candlestick updates for a single symbol.
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let stream_name = format!(
        "{}@kline_{}",
        unified_to_binance(symbol).to_lowercase(),
        interval_to_binance(interval)
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceFuturesWsKlineMsg = serde_json::from_value(json).ok()?;
        Some(raw.into_candle())
    })))
}

/// Stream mark price updates for a single symbol (1s interval).
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let stream_name = format!(
        "{}@markPrice@1s",
        unified_to_binance(symbol).to_lowercase()
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceWsMarkPriceRaw = serde_json::from_value(json).ok()?;
        Some(raw.into_mark_price())
    })))
}

/// Stream liquidation (force order) events for a single symbol.
pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let stream_name = format!(
        "{}@forceOrder",
        unified_to_binance(symbol).to_lowercase()
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceWsForceOrderMsg = serde_json::from_value(json).ok()?;
        Some(raw.into_liquidation())
    })))
}

// ---------------------------------------------------------------------------
// Combined (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream consistent top-1000 order-book updates for Binance Futures.
///
/// Аналог `spot::ws::stream_orderbooks_combined`, но с Futures-семантикой
/// sequence id: для каждого diff-события `pu` должен равняться `u` из
/// предыдущего события. Расхождение → re-sync.
pub async fn stream_orderbooks_combined(
    config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let all_streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@depth@100ms", unified_to_binance(s).to_lowercase()))
        .collect();

    let num_connections = all_streams.chunks(MAX_STREAMS_PER_CONNECTION).len();
    if num_connections > 1 {
        info!(
            "Binance Futures WS: sharding {} depth streams across {} connections",
            all_streams.len(),
            num_connections
        );
    }

    let rest = Arc::new(BinanceFuturesRest::new(config));
    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(COMBINED_WS_URL, chunk.to_vec()).await?;
        let shard_tx = out_tx.clone();
        let shard_rest = rest.clone();
        tokio::spawn(maintain_shard(raw, shard_tx, shard_rest));
    }

    Ok(Box::pin(ReceiverStream::new(out_rx)))
}

// ---------------------------------------------------------------------------
// Maintain logic (Futures-specific)
// ---------------------------------------------------------------------------

struct BufferedDiff {
    first_update_id: u64,
    last_update_id: u64,
    prev_update_id: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
    event_time: u64,
}

struct SymbolState {
    book: LocalOrderBook,
    buffer: VecDeque<BufferedDiff>,
    bootstrap_in_flight: bool,
    resync_count: u32,
    /// Подряд проваленные bootstrap'ы с момента последнего успешного sync.
    /// Используется для purge протухшей книги (Layer 2).
    failed_bootstraps: u32,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            book: LocalOrderBook::with_max_per_side(TOP_LEVELS),
            buffer: VecDeque::new(),
            bootstrap_in_flight: false,
            resync_count: 0,
            failed_bootstraps: 0,
        }
    }
}

struct SnapshotMsg {
    symbol: Symbol,
    last_update_id: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
}

async fn maintain_shard(
    mut raw: BoxStream<serde_json::Value>,
    out_tx: mpsc::Sender<OrderBook>,
    rest: Arc<BinanceFuturesRest>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SnapshotMsg>(256);

    loop {
        tokio::select! {
            biased;
            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("Binance Futures maintain shard: ws stream ended");
                    return;
                };
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &rest, &snap_tx).await {
                    debug!(error = %e, "binance futures ws event ignored");
                }
            }
            Some(snap) = snap_rx.recv() => {
                handle_snapshot(snap, &mut states, &out_tx).await;
            }
        }
    }
}

async fn handle_ws_event(
    json: &serde_json::Value,
    states: &mut HashMap<Symbol, SymbolState>,
    out_tx: &mpsc::Sender<OrderBook>,
    rest: &Arc<BinanceFuturesRest>,
    snap_tx: &mpsc::Sender<SnapshotMsg>,
) -> std::result::Result<(), &'static str> {
    let depth_json = json.get("data").unwrap_or(json);
    let raw: BinanceFuturesWsDepthRaw = serde_json::from_value(depth_json.clone())
        .map_err(|_| "parse depth")?;

    let symbol = binance_symbol_to_unified(&raw.symbol);
    let bids: Vec<(Decimal, Decimal)> =
        raw.bids.iter().filter_map(|p| parse_level(p)).collect();
    let asks: Vec<(Decimal, Decimal)> =
        raw.asks.iter().filter_map(|p| parse_level(p)).collect();

    let state = states.entry(symbol.clone()).or_insert_with(SymbolState::new);

    if !state.book.ready {
        if state.buffer.len() < MAX_BUFFER {
            state.buffer.push_back(BufferedDiff {
                first_update_id: raw.first_update_id,
                last_update_id: raw.last_update_id,
                prev_update_id: raw.prev_update_id,
                bids,
                asks,
                event_time: raw.event_time,
            });
        }
        if !state.bootstrap_in_flight {
            state.bootstrap_in_flight = true;
            spawn_bootstrap(symbol, rest.clone(), snap_tx.clone());
        }
        return Ok(());
    }

    // Futures sequence validation: pu == last_update_id предыдущего события.
    let lub = state.book.last_update_id;
    // Устаревшее событие (u <= lub) — молча дропаем. Такое случается,
    // когда WS event'ы отправлены биржей до REST snapshot'а, но получены
    // TCP-сокетом уже после snap_rx.recv().
    if raw.last_update_id <= lub {
        return Ok(());
    }
    let valid = raw.prev_update_id == lub;
    if !valid {
        state.resync_count += 1;
        warn!(
            symbol = %symbol,
            local_lub = lub,
            event_pu = raw.prev_update_id,
            event_u = raw.last_update_id,
            resync_count = state.resync_count,
            "Binance Futures: sequence gap, triggering re-sync"
        );
        state.book.ready = false;
        state.buffer.clear();
        state.buffer.push_back(BufferedDiff {
            first_update_id: raw.first_update_id,
            last_update_id: raw.last_update_id,
            prev_update_id: raw.prev_update_id,
            bids,
            asks,
            event_time: raw.event_time,
        });
        if !state.bootstrap_in_flight {
            state.bootstrap_in_flight = true;
            spawn_bootstrap(symbol, rest.clone(), snap_tx.clone());
        }
        return Ok(());
    }

    state.book.apply_diff(
        bids.iter().copied(),
        asks.iter().copied(),
        raw.last_update_id,
    );

    let ob = OrderBook {
        exchange: ExchangeId::BinanceFutures,
        symbol,
        bids: bids.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        asks: asks.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        timestamp_ms: raw.event_time,
        sequence: Some(raw.last_update_id),
    };
    let _ = out_tx.send(ob).await;
    Ok(())
}

async fn handle_snapshot(
    snap: SnapshotMsg,
    states: &mut HashMap<Symbol, SymbolState>,
    out_tx: &mpsc::Sender<OrderBook>,
) {
    let Some(state) = states.get_mut(&snap.symbol) else {
        return;
    };
    state.bootstrap_in_flight = false;

    let prev_book = if state.book.ready {
        return;
    } else {
        std::mem::take(&mut state.book)
    };

    let mut new_book = LocalOrderBook::with_max_per_side(TOP_LEVELS);
    new_book.set_snapshot(
        snap.bids.iter().copied(),
        snap.asks.iter().copied(),
        snap.last_update_id,
    );

    // Futures rule: drop events with u < snapshot.lastUpdateId.
    // Найти первый event где U <= snapshot.lastUpdateId AND u >= snapshot.lastUpdateId.
    let mut bootstrap_failed = false;
    let mut first_applied = false;
    let mut last_event_time = 0u64;
    while let Some(ev) = state.buffer.pop_front() {
        if ev.last_update_id < new_book.last_update_id {
            continue;
        }
        if !first_applied {
            // Первый event: U <= snap.lastUpdateId AND u >= snap.lastUpdateId.
            let lub = new_book.last_update_id;
            if !(ev.first_update_id <= lub && ev.last_update_id >= lub) {
                warn!(
                    symbol = %snap.symbol,
                    snap_lub = lub,
                    event_U = ev.first_update_id,
                    event_u = ev.last_update_id,
                    "Binance Futures: snapshot too old vs buffer, retrying"
                );
                bootstrap_failed = true;
                break;
            }
            first_applied = true;
        } else {
            // Последующие events: pu == prev event.u (новый last_update_id).
            if ev.prev_update_id != new_book.last_update_id {
                warn!(
                    symbol = %snap.symbol,
                    local_lub = new_book.last_update_id,
                    event_pu = ev.prev_update_id,
                    "Binance Futures: buffered event chain broken, re-syncing"
                );
                bootstrap_failed = true;
                break;
            }
        }
        last_event_time = ev.event_time;
        new_book.apply_diff(
            ev.bids.iter().copied(),
            ev.asks.iter().copied(),
            ev.last_update_id,
        );
    }

    if bootstrap_failed {
        state.failed_bootstraps += 1;
        // Layer 2: после нескольких подряд провалов книга безнадёжно
        // протухла. Эмитим qty=0 по ранее отданным уровням, чтобы
        // density-engine снял зомби (иначе он держит старый qty до
        // следующего успешного re-sync). Покрываем только пересечение
        // момента — purge один раз при достижении порога.
        if state.failed_bootstraps == PURGE_AFTER_FAILURES && prev_book.last_update_id > 0 {
            let empty = LocalOrderBook::with_max_per_side(TOP_LEVELS);
            let (bids, asks) = empty.diff_against_prev(&prev_book, TOP_LEVELS);
            if !bids.is_empty() || !asks.is_empty() {
                warn!(
                    symbol = %snap.symbol,
                    failed = state.failed_bootstraps,
                    "Binance Futures: purging stale book after repeated bootstrap failures"
                );
                let ob = OrderBook {
                    exchange: ExchangeId::BinanceFutures,
                    symbol: snap.symbol.clone(),
                    bids,
                    asks,
                    timestamp_ms: 0,
                    sequence: Some(prev_book.last_update_id),
                };
                let _ = out_tx.send(ob).await;
            }
        }
        state.book = prev_book;
        state.buffer.clear();
        return;
    }

    info!(
        symbol = %snap.symbol,
        levels_bids = new_book.bids.len(),
        levels_asks = new_book.asks.len(),
        last_update_id = new_book.last_update_id,
        was_resync = prev_book.last_update_id > 0,
        "Binance Futures: local book initialized"
    );
    state.book = new_book;
    state.resync_count = 0;
    state.failed_bootstraps = 0;

    let ts = if last_event_time > 0 { last_event_time } else { 0 };
    let ob = if prev_book.last_update_id > 0 {
        let (bids, asks) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
        OrderBook {
            exchange: ExchangeId::BinanceFutures,
            symbol: snap.symbol.clone(),
            bids,
            asks,
            timestamp_ms: ts,
            sequence: Some(state.book.last_update_id),
        }
    } else {
        state.book.to_orderbook(
            ExchangeId::BinanceFutures,
            snap.symbol.clone(),
            ts,
            TOP_LEVELS,
        )
    };
    let _ = out_tx.send(ob).await;
}

fn spawn_bootstrap(
    symbol: Symbol,
    rest: Arc<BinanceFuturesRest>,
    snap_tx: mpsc::Sender<SnapshotMsg>,
) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            futures_limiter().acquire(DEPTH_WEIGHT).await;
            match rest.orderbook(&symbol, 1000).await {
                Ok(ob) => {
                    let bids: Vec<(Decimal, Decimal)> =
                        ob.bids.iter().map(|l| (l.price, l.qty)).collect();
                    let asks: Vec<(Decimal, Decimal)> =
                        ob.asks.iter().map(|l| (l.price, l.qty)).collect();
                    let Some(seq) = ob.sequence else {
                        warn!(symbol = %symbol, "Binance Futures snapshot missing sequence, retrying");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        continue;
                    };
                    let _ = snap_tx
                        .send(SnapshotMsg {
                            symbol,
                            last_update_id: seq,
                            bids,
                            asks,
                        })
                        .await;
                    return;
                }
                Err(e) => {
                    warn!(symbol = %symbol, error = %e, backoff_ms = backoff.as_millis() as u64, "Binance Futures REST snapshot failed");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}

fn parse_level(raw: &[String; 2]) -> Option<(Decimal, Decimal)> {
    let price = Decimal::from_str(&raw[0]).ok()?;
    let qty = Decimal::from_str(&raw[1]).ok()?;
    Some((price, qty))
}

/// Stream real-time trades for multiple symbols, automatically
/// sharding subscriptions across several WebSocket connections to stay
/// within Binance limits and avoid connection resets.
///
/// Uses the `/stream` endpoint with SUBSCRIBE method instead of URL query
/// params to avoid URL-length issues with many symbols.
pub async fn stream_trades_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let all_streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@trade", unified_to_binance(s).to_lowercase()))
        .collect();

    let num_connections = all_streams.chunks(MAX_STREAMS_PER_CONNECTION).len();
    if num_connections > 1 {
        info!(
            "Binance Futures WS: sharding {} trade streams across {} connections",
            all_streams.len(),
            num_connections
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(COMBINED_WS_URL, chunk.to_vec()).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.filter_map(|json| async move {
            let data = json.get("data")?.clone();
            let raw: BinanceFuturesWsTradeRaw = serde_json::from_value(data).ok()?;
            Some(raw.into_trade())
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}

#[cfg(test)]
mod resync_tests {
    //! Repro tests for the "moved wall zombie" (TON/USDT ask@1.7638 = 476k
    //! while the real book had 3141.9 there, the wall having repriced to
    //! 1.7650). These exercise `handle_snapshot` — the re-sync path — which
    //! is the only emit path besides per-diff passthrough.
    use super::*;
    use rust_decimal_macros::dec;

    fn ston() -> Symbol {
        Symbol::new("TON", "USDT")
    }

    /// Build a SymbolState whose book is *stale* (still has the old wall) and
    /// marked not-ready — i.e. the exact state after a `sequence gap` is
    /// detected (handle_ws_event sets `book.ready = false` but keeps data).
    fn stale_state(last_update_id: u64) -> SymbolState {
        let mut st = SymbolState::new();
        st.book.set_snapshot(
            vec![(dec!(1.7593), dec!(372.7))],
            // The phantom wall the exchange already moved away from:
            vec![(dec!(1.7594), dec!(514.1)), (dec!(1.7638), dec!(476833.3))],
            last_update_id,
        );
        st.book.ready = false; // post-gap
        st
    }

    /// A SUCCESSFUL re-sync (fresh REST snapshot, no buffered chain to bridge)
    /// MUST emit a diff that corrects the moved wall down to its real qty.
    /// This is the path that DOES kill the zombie — it works when bootstrap
    /// succeeds.
    #[tokio::test]
    async fn resync_success_corrects_moved_wall() {
        let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
        states.insert(ston(), stale_state(1000));

        let (out_tx, mut out_rx) = mpsc::channel::<OrderBook>(16);

        // Fresh REST: wall is gone from 1.7638 (only dust left), real 460k
        // now sits at 1.7650.
        let snap = SnapshotMsg {
            symbol: ston(),
            last_update_id: 2000,
            bids: vec![(dec!(1.7593), dec!(372.7))],
            asks: vec![
                (dec!(1.7594), dec!(514.1)),
                (dec!(1.7638), dec!(3141.9)),
                (dec!(1.7650), dec!(459812.7)),
            ],
        };

        handle_snapshot(snap, &mut states, &out_tx).await;

        // Local book is now fresh.
        let book = &states[&ston()].book;
        assert_eq!(book.asks.get(&dec!(1.7638)), Some(&dec!(3141.9)));
        assert!(book.ready);

        // And downstream got a diff that carries the correction for 1.7638.
        let ob = out_rx.try_recv().expect("expected a re-sync diff emit");
        let corrected = ob.asks.iter().find(|l| l.price == dec!(1.7638));
        assert_eq!(
            corrected.map(|l| l.qty),
            Some(dec!(3141.9)),
            "successful re-sync must emit the corrected qty for the moved wall"
        );
    }

    /// A FAILED re-sync (buffered chain broken — exactly the
    /// "buffered event chain broken, re-syncing" warn we see flooding TON in
    /// prod) restores the STALE book and emits NOTHING. The phantom wall
    /// survives and downstream is never told the book went stale. This is the
    /// window in which the zombie lives — and on hyper-active symbols these
    /// failures repeat for minutes (1635 re-syncs / 30 min in prod).
    #[tokio::test]
    async fn resync_chain_broken_retains_stale_zombie() {
        let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
        let mut st = stale_state(1000);
        // Buffer: ev1 bridges the snapshot (U<=lub<=u), ev2 breaks the chain
        // (pu != prev u) — the bootstrap aborts.
        st.buffer.push_back(BufferedDiff {
            first_update_id: 1000,
            last_update_id: 1005,
            prev_update_id: 999,
            bids: vec![],
            asks: vec![],
            event_time: 1,
        });
        st.buffer.push_back(BufferedDiff {
            first_update_id: 9000,
            last_update_id: 9005,
            prev_update_id: 9999, // != 1005 -> chain broken
            bids: vec![],
            asks: vec![],
            event_time: 2,
        });
        states.insert(ston(), st);

        let (out_tx, mut out_rx) = mpsc::channel::<OrderBook>(16);

        // Fresh REST that WOULD have corrected the wall — but it's discarded.
        let snap = SnapshotMsg {
            symbol: ston(),
            last_update_id: 1000,
            bids: vec![(dec!(1.7593), dec!(372.7))],
            asks: vec![(dec!(1.7638), dec!(3141.9)), (dec!(1.7650), dec!(459812.7))],
        };

        handle_snapshot(snap, &mut states, &out_tx).await;

        // BUG: the stale phantom wall is still in the book...
        let book = &states[&ston()].book;
        assert_eq!(
            book.asks.get(&dec!(1.7638)),
            Some(&dec!(476833.3)),
            "failed bootstrap restores the stale wall (zombie persists)"
        );
        // ...and NOTHING was emitted downstream to correct it.
        assert!(
            out_rx.try_recv().is_err(),
            "failed bootstrap emits no diff — downstream keeps the zombie qty"
        );
    }

    /// Layer 2: after PURGE_AFTER_FAILURES consecutive bootstrap failures, the
    /// stale book is purged downstream (qty=0 for previously-emitted levels) so
    /// density-engine drops the zombie instead of waiting for a re-sync that may
    /// be minutes away on a hot symbol.
    #[tokio::test]
    async fn resync_purges_stale_book_after_repeated_failures() {
        let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
        states.insert(ston(), stale_state(1000));

        let (out_tx, mut out_rx) = mpsc::channel::<OrderBook>(16);

        let mut purge: Option<OrderBook> = None;
        for _ in 0..PURGE_AFTER_FAILURES {
            // Re-arm a broken buffer each round (handle_snapshot clears it on fail).
            {
                let st = states.get_mut(&ston()).unwrap();
                st.buffer.clear();
                st.buffer.push_back(BufferedDiff {
                    first_update_id: 1000,
                    last_update_id: 1005,
                    prev_update_id: 999,
                    bids: vec![],
                    asks: vec![],
                    event_time: 1,
                });
                st.buffer.push_back(BufferedDiff {
                    first_update_id: 9000,
                    last_update_id: 9005,
                    prev_update_id: 9999, // chain broken
                    bids: vec![],
                    asks: vec![],
                    event_time: 2,
                });
            }
            let snap = SnapshotMsg {
                symbol: ston(),
                last_update_id: 1000,
                bids: vec![(dec!(1.7593), dec!(372.7))],
                asks: vec![(dec!(1.7638), dec!(3141.9))],
            };
            handle_snapshot(snap, &mut states, &out_tx).await;
            while let Ok(ob) = out_rx.try_recv() {
                purge = Some(ob);
            }
        }

        let purge = purge.expect("expected a purge emit after repeated failures");
        let level = purge
            .asks
            .iter()
            .find(|l| l.price == dec!(1.7638))
            .expect("purge must cover the stale wall price");
        assert_eq!(
            level.qty,
            Decimal::ZERO,
            "purge must emit qty=0 to delete the stale wall downstream"
        );
        assert_eq!(states[&ston()].failed_bootstraps, PURGE_AFTER_FAILURES);
    }
}
