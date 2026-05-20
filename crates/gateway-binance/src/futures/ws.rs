use crate::futures::mapper::*;
use crate::futures::rest::BinanceFuturesRest;
use crate::local_book::LocalOrderBook;
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

const TOP_LEVELS: usize = 1000;
const MAX_BUFFER: usize = 1024;
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

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);
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
                                    if tx.send(json).await.is_err() {
                                        break 'outer;
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
}

impl SymbolState {
    fn new() -> Self {
        Self {
            book: LocalOrderBook::new(),
            buffer: VecDeque::new(),
            bootstrap_in_flight: false,
            resync_count: 0,
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

    let mut new_book = LocalOrderBook::new();
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
