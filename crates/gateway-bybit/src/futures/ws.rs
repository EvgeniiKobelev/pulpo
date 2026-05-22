use crate::futures::mapper::*;
use crate::futures::rest::BybitLinearRest;
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

const WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";

/// Для linear (perp) Bybit максимум — `orderbook.200`. Каналы `orderbook.500`
/// и `orderbook.1000` доступны только spot'у (на linear возвращают
/// `error:handler not found`). 200 уровней покрывают ±5% от mid на большинстве
/// liquid futures pair'ов.
const TOP_LEVELS: usize = 200;
const MAX_BUFFER: usize = 1024;
const SUBSCRIBE_CHUNK: usize = 10;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

async fn subscribe_and_stream(
    topics: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BybitFutures,
                message: e.to_string(),
            })?;

    let (mut write, read) = ws_stream.split();

    for chunk in topics.chunks(SUBSCRIBE_CHUNK) {
        let sub = serde_json::json!({"op": "subscribe", "args": chunk});
        write
            .send(Message::text(sub.to_string()))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BybitFutures,
                message: e.to_string(),
            })?;
    }

    let (tx, rx) = mpsc::channel::<serde_json::Value>(2048);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            // См. spot/ws.rs — periodic re-subscribe для refresh stale levels.
            let mut resub_interval = tokio::time::interval(Duration::from_secs(300));
            resub_interval.tick().await;

            loop {
                tokio::select! {
                    _ = resub_interval.tick() => {
                        debug!("Bybit Futures WS: periodic resubscribe to refresh stale levels");
                        let mut all_ok = true;
                        for chunk in topics.chunks(SUBSCRIBE_CHUNK) {
                            let sub = serde_json::json!({"op":"subscribe","args":chunk});
                            if write.send(Message::text(sub.to_string())).await.is_err() {
                                all_ok = false;
                                break;
                            }
                        }
                        if !all_ok {
                            break;
                        }
                    }
                    msg_result = read.next() => {
                        match msg_result {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("op").and_then(|v| v.as_str()) == Some("ping") {
                                        let pong = serde_json::json!({"op": "pong"});
                                        let _ = write.send(Message::text(pong.to_string())).await;
                                        continue;
                                    }
                                    if matches!(
                                        json.get("op").and_then(|v| v.as_str()),
                                        Some("subscribe") | Some("pong")
                                    ) {
                                        continue;
                                    }
                                    if json.get("topic").is_some()
                                        && tx.send(json).await.is_err()
                                    {
                                        break 'outer;
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Bybit Futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Bybit Futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Bybit Futures WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Bybit Futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut sub_ok = true;
                        for chunk in topics.chunks(SUBSCRIBE_CHUNK) {
                            let sub = serde_json::json!({"op": "subscribe", "args": chunk});
                            if new_write
                                .send(Message::text(sub.to_string()))
                                .await
                                .is_err()
                            {
                                sub_ok = false;
                                break;
                            }
                        }
                        if !sub_ok {
                            warn!("Bybit Futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Bybit Futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Bybit Futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Bybit Futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbook(
    config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    stream_orderbooks_batch(config, std::slice::from_ref(symbol)).await
}

pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let topic = format!("publicTrade.{}", unified_to_bybit(symbol));
    let raw_stream = subscribe_and_stream(vec![topic]).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<BybitLinearWsTrade> =
                    serde_json::from_value(data.clone()).ok()?;
                if !trades.is_empty() {
                    let converted: Vec<Trade> =
                        trades.into_iter().map(|t| t.into_trade()).collect();
                    return Some((futures::stream::iter(converted), stream));
                }
            }
        })
        .flatten(),
    ))
}

pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let topic = format!(
        "kline.{}.{}",
        interval_to_bybit(interval),
        unified_to_bybit(symbol)
    );
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![topic]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?;
            let klines: Vec<BybitLinearWsKlineData> =
                serde_json::from_value(data.clone()).ok()?;
            let kline = klines.into_iter().next()?;
            Some(kline.into_candle(sym))
        }
    })))
}

pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let topic = format!("tickers.{}", unified_to_bybit(symbol));
    let raw_stream = subscribe_and_stream(vec![topic]).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let ts = json.get("ts")?.as_u64()?;
        let data = json.get("data")?;
        let raw: BybitLinearWsTicker = serde_json::from_value(data.clone()).ok()?;
        Some(raw.into_mark_price(ts))
    })))
}

pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let topic = format!("liquidation.{}", unified_to_bybit(symbol));
    let raw_stream = subscribe_and_stream(vec![topic]).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let data = json.get("data")?;
        let raw: BybitWsLiquidation = serde_json::from_value(data.clone()).ok()?;
        Some(raw.into_liquidation())
    })))
}

// ---------------------------------------------------------------------------
// Batch streams
// ---------------------------------------------------------------------------

/// Стрим консистентного стакана (top-200) для linear/perp символов.
/// Логика идентична spot/ws.rs::stream_orderbooks_batch, разница только
/// в WS URL (`/v5/public/linear`) и максимальной глубине (200 вместо 1000).
pub async fn stream_orderbooks_batch(
    config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let topics: Vec<String> = symbols
        .iter()
        .map(|s| format!("orderbook.200.{}", unified_to_bybit(s)))
        .collect();
    let raw = subscribe_and_stream(topics).await?;

    let rest = Arc::new(BybitLinearRest::new(config));
    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    tokio::spawn(maintain_shard(raw, out_tx, rest));

    Ok(Box::pin(ReceiverStream::new(out_rx)))
}

pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let topics: Vec<String> = symbols
        .iter()
        .map(|s| format!("publicTrade.{}", unified_to_bybit(s)))
        .collect();
    let raw_stream = subscribe_and_stream(topics).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<BybitLinearWsTrade> =
                    serde_json::from_value(data.clone()).ok()?;
                if !trades.is_empty() {
                    let converted: Vec<Trade> =
                        trades.into_iter().map(|t| t.into_trade()).collect();
                    return Some((futures::stream::iter(converted), stream));
                }
            }
        })
        .flatten(),
    ))
}

// ---------------------------------------------------------------------------
// Maintain logic
// ---------------------------------------------------------------------------

struct BufferedDelta {
    u: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
    event_time: u64,
}

struct SymbolState {
    book: LocalOrderBook,
    last_u: u64,
    buffer: VecDeque<BufferedDelta>,
    bootstrap_in_flight: bool,
    resync_count: u32,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            book: LocalOrderBook::new(),
            last_u: 0,
            buffer: VecDeque::new(),
            bootstrap_in_flight: false,
            resync_count: 0,
        }
    }
}

struct SnapshotMsg {
    symbol: Symbol,
    last_u: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
}

async fn maintain_shard(
    mut raw: BoxStream<serde_json::Value>,
    out_tx: mpsc::Sender<OrderBook>,
    rest: Arc<BybitLinearRest>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SnapshotMsg>(256);

    loop {
        tokio::select! {
            biased;

            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("Bybit Futures maintain shard: ws stream ended");
                    return;
                };
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &rest, &snap_tx).await {
                    debug!(error = %e, "bybit futures ws event ignored");
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
    rest: &Arc<BybitLinearRest>,
    snap_tx: &mpsc::Sender<SnapshotMsg>,
) -> std::result::Result<(), &'static str> {
    let msg_type = json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let event_time: u64 = json.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);

    let data = json.get("data").ok_or("no data")?;
    let book: BybitLinearWsOrderBook =
        serde_json::from_value(data.clone()).map_err(|_| "parse book")?;

    let symbol = bybit_symbol_to_unified(&book.s);
    let bids: Vec<(Decimal, Decimal)> = book
        .b
        .iter()
        .filter_map(|p| parse_level(p))
        .collect();
    let asks: Vec<(Decimal, Decimal)> = book
        .a
        .iter()
        .filter_map(|p| parse_level(p))
        .collect();
    let u = book.u;

    let state = states.entry(symbol.clone()).or_insert_with(SymbolState::new);

    if msg_type == "snapshot" || u == 1 {
        let mut new_book = LocalOrderBook::new();
        new_book.set_snapshot(bids.iter().copied(), asks.iter().copied(), u);

        let prev_book = std::mem::take(&mut state.book);
        state.book = new_book;
        state.last_u = u;
        state.buffer.clear();
        state.bootstrap_in_flight = false;

        info!(
            symbol = %symbol,
            levels_bids = state.book.bids.len(),
            levels_asks = state.book.asks.len(),
            u,
            was_resync = prev_book.last_update_id > 0,
            kind = msg_type,
            "Bybit Futures: local book initialized"
        );

        let ob = if prev_book.last_update_id > 0 {
            let (b, a) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
            OrderBook {
                exchange: ExchangeId::BybitFutures,
                symbol,
                bids: b,
                asks: a,
                timestamp_ms: event_time,
                sequence: Some(u),
            }
        } else {
            state
                .book
                .to_orderbook(ExchangeId::BybitFutures, symbol, event_time, TOP_LEVELS)
        };
        let _ = out_tx.send(ob).await;
        return Ok(());
    }

    if !state.book.ready {
        if state.buffer.len() < MAX_BUFFER {
            state.buffer.push_back(BufferedDelta {
                u,
                bids,
                asks,
                event_time,
            });
        }
        if !state.bootstrap_in_flight {
            state.bootstrap_in_flight = true;
            spawn_bootstrap(symbol.clone(), rest.clone(), snap_tx.clone());
        }
        return Ok(());
    }

    if u <= state.last_u {
        return Ok(());
    }

    if u != state.last_u + 1 {
        state.resync_count += 1;
        warn!(
            symbol = %symbol,
            local_u = state.last_u,
            event_u = u,
            resync_count = state.resync_count,
            "Bybit Futures: sequence gap, triggering re-sync"
        );
        state.book.ready = false;
        state.buffer.clear();
        state.buffer.push_back(BufferedDelta {
            u,
            bids,
            asks,
            event_time,
        });
        if !state.bootstrap_in_flight {
            state.bootstrap_in_flight = true;
            spawn_bootstrap(symbol, rest.clone(), snap_tx.clone());
        }
        return Ok(());
    }

    state.book.apply_diff(bids.iter().copied(), asks.iter().copied(), u);
    state.last_u = u;

    let ob = OrderBook {
        exchange: ExchangeId::BybitFutures,
        symbol,
        bids: bids.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        asks: asks.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        timestamp_ms: event_time,
        sequence: Some(u),
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

    if state.book.ready {
        return;
    }

    let prev_book = std::mem::take(&mut state.book);
    let mut new_book = LocalOrderBook::new();
    new_book.set_snapshot(
        snap.bids.iter().copied(),
        snap.asks.iter().copied(),
        snap.last_u,
    );

    let mut last_event_time = 0u64;
    while let Some(ev) = state.buffer.pop_front() {
        if ev.u <= new_book.last_update_id {
            continue;
        }
        last_event_time = ev.event_time;
        new_book.apply_diff(
            ev.bids.iter().copied(),
            ev.asks.iter().copied(),
            ev.u,
        );
    }

    info!(
        symbol = %snap.symbol,
        levels_bids = new_book.bids.len(),
        levels_asks = new_book.asks.len(),
        last_u = new_book.last_update_id,
        was_resync = prev_book.last_update_id > 0,
        "Bybit Futures: REST bootstrap applied"
    );
    state.last_u = new_book.last_update_id;
    state.book = new_book;
    state.resync_count = 0;

    let ts = if last_event_time > 0 { last_event_time } else { 0 };
    let ob = if prev_book.last_update_id > 0 {
        let (bids, asks) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
        OrderBook {
            exchange: ExchangeId::BybitFutures,
            symbol: snap.symbol.clone(),
            bids,
            asks,
            timestamp_ms: ts,
            sequence: Some(state.book.last_update_id),
        }
    } else {
        state
            .book
            .to_orderbook(ExchangeId::BybitFutures, snap.symbol.clone(), ts, TOP_LEVELS)
    };
    let _ = out_tx.send(ob).await;
}

fn spawn_bootstrap(
    symbol: Symbol,
    rest: Arc<BybitLinearRest>,
    snap_tx: mpsc::Sender<SnapshotMsg>,
) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            // Bybit linear REST max limit = 500.
            match rest.orderbook(&symbol, 500).await {
                Ok(ob) => {
                    let bids: Vec<(Decimal, Decimal)> =
                        ob.bids.iter().map(|l| (l.price, l.qty)).collect();
                    let asks: Vec<(Decimal, Decimal)> =
                        ob.asks.iter().map(|l| (l.price, l.qty)).collect();
                    let Some(seq) = ob.sequence else {
                        warn!(symbol = %symbol, "Bybit Futures snapshot missing sequence, retrying");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        continue;
                    };
                    let _ = snap_tx
                        .send(SnapshotMsg {
                            symbol,
                            last_u: seq,
                            bids,
                            asks,
                        })
                        .await;
                    return;
                }
                Err(e) => {
                    warn!(
                        symbol = %symbol,
                        error = %e,
                        backoff_ms = backoff.as_millis() as u64,
                        "Bybit Futures REST snapshot failed"
                    );
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
