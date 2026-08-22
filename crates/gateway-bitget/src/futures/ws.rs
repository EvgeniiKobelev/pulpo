use crate::futures::mapper::*;
use crate::futures::rest::BitgetFuturesRest;
use futures::{stream::SelectAll, SinkExt, StreamExt};
use gateway_core::*;
use rust_decimal::Decimal;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://ws.bitget.com/v2/ws/public";

/// Канал `books` Bitget futures на снапшоте даёт 500 уровней.
const TOP_LEVELS: usize = 500;
const MAX_BUFFER: usize = 1024;
const MAX_ARGS_PER_CONNECTION: usize = 40;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// `periodic_resub` — см. spot/ws.rs: 5-мин re-subscribe нужен только `books`;
/// на `trade` каждый subscribe репушит снапшот последних трейдов → дубли.
async fn subscribe_and_stream(
    args: Vec<serde_json::Value>,
    periodic_resub: bool,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) = connect_async(WS_URL)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: ExchangeId::BitgetFutures,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
    write
        .send(Message::text(sub.to_string()))
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: ExchangeId::BitgetFutures,
            message: e.to_string(),
        })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(2048);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            let _ = write.send(Message::text("ping".to_string())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
            ping_interval.tick().await;

            // См. spot/ws.rs — тот же механизм: каждые 5 мин принудительно
            // re-subscribe, Bitget отвечает свежим snapshot'ом, pulpo через
            // diff_against_prev обновляет stale-уровни в LocalOrderBook.
            let mut resub_interval = tokio::time::interval(Duration::from_secs(300));
            resub_interval.tick().await;

            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text("ping".to_string())).await.is_err() {
                            break;
                        }
                    }
                    _ = resub_interval.tick(), if periodic_resub => {
                        debug!("Bitget futures WS: periodic resubscribe to refresh stale levels");
                        let sub = serde_json::json!({"op":"subscribe","args":args.clone()});
                        if write.send(Message::text(sub.to_string())).await.is_err() {
                            break;
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if text == "pong" {
                                    continue;
                                }
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("event").and_then(|v| v.as_str()) == Some("subscribe") {
                                        debug!("Bitget futures WS subscribed: {}", text);
                                        continue;
                                    }
                                    if json.get("event").and_then(|v| v.as_str()) == Some("error") {
                                        warn!("Bitget futures WS error event: {}", text);
                                        continue;
                                    }
                                    if json.get("data").is_some()
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
                                warn!("Bitget futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Bitget futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Bitget futures WS stream ended unexpectedly");
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
                warn!("Bitget futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
                        if new_write
                            .send(Message::text(sub.to_string()))
                            .await
                            .is_err()
                        {
                            warn!("Bitget futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Bitget futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Bitget futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Bitget futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

fn sub_arg(channel: &str, inst_id: &str) -> serde_json::Value {
    serde_json::json!({
        "instType": "USDT-FUTURES",
        "channel": channel,
        "instId": inst_id
    })
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

pub async fn stream_trades(_config: &ExchangeConfig, symbol: &Symbol) -> Result<BoxStream<Trade>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("trade", &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![arg], false).await?;

    Ok(Box::pin(
        futures::stream::unfold((raw_stream, sym), |(mut stream, sym)| async move {
            loop {
                let json = stream.next().await?;
                // Снапшот последних трейдов при (пере)подписке — не эмитим (дубли).
                if json.get("action").and_then(|v| v.as_str()) == Some("snapshot") {
                    continue;
                }
                let data = json.get("data")?;
                if let Ok(trades) = serde_json::from_value::<Vec<BitgetMixWsTradeRaw>>(data.clone())
                {
                    if !trades.is_empty() {
                        let converted: Vec<Trade> = trades
                            .into_iter()
                            .map(|t| t.into_trade(sym.clone()))
                            .collect();
                        return Some((futures::stream::iter(converted), (stream, sym)));
                    }
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
    let inst_id = unified_to_bitget(symbol);
    let channel = interval_to_bitget_ws(interval);
    let arg = sub_arg(channel, &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![arg], false).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?.as_array()?;
            let row: Vec<String> = first
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            parse_ws_kline(&row, sym)
        }
    })))
}

// ---------------------------------------------------------------------------
// Futures-specific streams
// ---------------------------------------------------------------------------

pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("ticker", &inst_id);
    let raw_stream = subscribe_and_stream(vec![arg], false).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| async move {
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: BitgetMixWsTickerRaw = serde_json::from_value(first.clone()).ok()?;
        Some(raw.into_mark_price())
    })))
}

pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("liquidation", &inst_id);
    let raw_stream = subscribe_and_stream(vec![arg], false).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| async move {
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: BitgetMixWsLiquidationRaw = serde_json::from_value(first.clone()).ok()?;
        Some(raw.into_liquidation())
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams — с поддержкой LocalOrderBook
// ---------------------------------------------------------------------------

/// См. spot/ws.rs::stream_orderbooks_batch — та же логика, разница только
/// в instType=USDT-FUTURES и используется REST `BitgetFuturesRest`.
pub async fn stream_orderbooks_batch(
    config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let all_args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("books", &unified_to_bitget(s)))
        .collect();

    let num_connections = all_args.chunks(MAX_ARGS_PER_CONNECTION).len();
    if num_connections > 1 {
        info!(
            "Bitget Futures WS: sharding {} books streams across {} connections",
            all_args.len(),
            num_connections
        );
    }

    let rest = Arc::new(BitgetFuturesRest::new(config));
    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    for chunk in all_args.chunks(MAX_ARGS_PER_CONNECTION) {
        let raw = subscribe_and_stream(chunk.to_vec(), true).await?;
        let shard_tx = out_tx.clone();
        let shard_rest = rest.clone();
        tokio::spawn(maintain_shard(raw, shard_tx, shard_rest));
    }

    Ok(Box::pin(ReceiverStream::new(out_rx)))
}

pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    const CHUNK_SIZE: usize = 50;

    let mut all = SelectAll::new();
    for chunk in symbols.chunks(CHUNK_SIZE) {
        let args: Vec<serde_json::Value> = chunk
            .iter()
            .map(|s| sub_arg("trade", &unified_to_bitget(s)))
            .collect();
        let raw_stream = subscribe_and_stream(args, false).await?;
        let chunk_stream: BoxStream<Trade> = Box::pin(
            futures::stream::unfold(raw_stream, |mut stream| async move {
                loop {
                    let json = stream.next().await?;
                    // Снапшот последних трейдов при (пере)подписке — не эмитим (дубли).
                    if json.get("action").and_then(|v| v.as_str()) == Some("snapshot") {
                        continue;
                    }
                    let arg = json.get("arg")?;
                    let inst_id = arg.get("instId")?.as_str()?;
                    let symbol = bitget_symbol_to_unified(inst_id);
                    let data = json.get("data")?;
                    if let Ok(trades) =
                        serde_json::from_value::<Vec<BitgetMixWsTradeRaw>>(data.clone())
                    {
                        if !trades.is_empty() {
                            let converted: Vec<Trade> = trades
                                .into_iter()
                                .map(|t| t.into_trade(symbol.clone()))
                                .collect();
                            return Some((futures::stream::iter(converted), stream));
                        }
                    }
                }
            })
            .flatten(),
        );
        all.push(chunk_stream);
    }
    Ok(Box::pin(all))
}

// ---------------------------------------------------------------------------
// Maintain logic
// ---------------------------------------------------------------------------

struct BufferedUpdate {
    seq: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
    event_time: u64,
}

struct SymbolState {
    book: LocalOrderBook,
    last_seq: u64,
    buffer: VecDeque<BufferedUpdate>,
    bootstrap_in_flight: bool,
    resync_count: u32,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            book: LocalOrderBook::with_max_per_side(TOP_LEVELS),
            last_seq: 0,
            buffer: VecDeque::new(),
            bootstrap_in_flight: false,
            resync_count: 0,
        }
    }
}

struct SnapshotMsg {
    symbol: Symbol,
    last_seq: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
}

async fn maintain_shard(
    mut raw: BoxStream<serde_json::Value>,
    out_tx: mpsc::Sender<OrderBook>,
    rest: Arc<BitgetFuturesRest>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SnapshotMsg>(256);

    loop {
        tokio::select! {
            biased;

            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("Bitget Futures maintain shard: ws stream ended");
                    return;
                };
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &rest, &snap_tx).await {
                    debug!(error = %e, "bitget futures ws event ignored");
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
    rest: &Arc<BitgetFuturesRest>,
    snap_tx: &mpsc::Sender<SnapshotMsg>,
) -> std::result::Result<(), &'static str> {
    let arg = json.get("arg").ok_or("no arg")?;
    let inst_id = arg
        .get("instId")
        .and_then(|v| v.as_str())
        .ok_or("no instId")?;
    let symbol = bitget_symbol_to_unified(inst_id);

    let action = json.get("action").and_then(|v| v.as_str()).unwrap_or("");

    let data_arr = json
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or("no data array")?;
    let first = data_arr.first().ok_or("empty data")?;
    let book_data: BitgetMixWsOrderBook =
        serde_json::from_value(first.clone()).map_err(|_| "parse book data")?;

    let bids = parse_book_levels(&book_data.bids);
    let asks = parse_book_levels(&book_data.asks);
    let event_time: u64 = book_data.ts.parse().unwrap_or(0);
    let seq = book_data.seq.ok_or("no seq")?;
    let pseq = book_data.pseq.unwrap_or(0);

    let state = states
        .entry(symbol.clone())
        .or_insert_with(SymbolState::new);

    if action == "snapshot" {
        let mut new_book = LocalOrderBook::with_max_per_side(TOP_LEVELS);
        new_book.set_snapshot(bids.iter().copied(), asks.iter().copied(), seq);

        let prev_book = std::mem::take(&mut state.book);
        state.book = new_book;
        state.last_seq = seq;
        state.buffer.clear();
        state.bootstrap_in_flight = false;

        info!(
            symbol = %symbol,
            levels_bids = state.book.bids.len(),
            levels_asks = state.book.asks.len(),
            seq,
            was_resync = prev_book.last_update_id > 0,
            "Bitget Futures: local book initialized"
        );

        let ob = if prev_book.last_update_id > 0 {
            let (b, a) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
            OrderBook {
                exchange: ExchangeId::BitgetFutures,
                symbol,
                bids: b,
                asks: a,
                timestamp_ms: event_time,
                sequence: Some(seq),
            }
        } else {
            state
                .book
                .to_orderbook(ExchangeId::BitgetFutures, symbol, event_time, TOP_LEVELS)
        };
        let _ = out_tx.send(ob).await;
        return Ok(());
    }

    if !state.book.ready {
        if state.buffer.len() < MAX_BUFFER {
            state.buffer.push_back(BufferedUpdate {
                seq,
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

    if seq <= state.last_seq {
        return Ok(());
    }

    if pseq != state.last_seq {
        state.resync_count += 1;
        warn!(
            symbol = %symbol,
            local_seq = state.last_seq,
            event_pseq = pseq,
            event_seq = seq,
            resync_count = state.resync_count,
            "Bitget Futures: sequence gap, triggering re-sync"
        );
        state.book.ready = false;
        state.buffer.clear();
        state.buffer.push_back(BufferedUpdate {
            seq,
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

    state
        .book
        .apply_diff(bids.iter().copied(), asks.iter().copied(), seq);
    state.last_seq = seq;

    let ob = OrderBook {
        exchange: ExchangeId::BitgetFutures,
        symbol,
        bids: bids.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        asks: asks.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        timestamp_ms: event_time,
        sequence: Some(seq),
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
    let mut new_book = LocalOrderBook::with_max_per_side(TOP_LEVELS);
    new_book.set_snapshot(
        snap.bids.iter().copied(),
        snap.asks.iter().copied(),
        snap.last_seq,
    );

    let mut last_event_time = 0u64;
    while let Some(ev) = state.buffer.pop_front() {
        if ev.seq <= new_book.last_update_id {
            continue;
        }
        last_event_time = ev.event_time;
        new_book.apply_diff(ev.bids.iter().copied(), ev.asks.iter().copied(), ev.seq);
    }

    info!(
        symbol = %snap.symbol,
        levels_bids = new_book.bids.len(),
        levels_asks = new_book.asks.len(),
        last_seq = new_book.last_update_id,
        was_resync = prev_book.last_update_id > 0,
        "Bitget Futures: REST bootstrap applied"
    );
    state.last_seq = new_book.last_update_id;
    state.book = new_book;
    state.resync_count = 0;

    let ts = if last_event_time > 0 {
        last_event_time
    } else {
        0
    };
    let ob = if prev_book.last_update_id > 0 {
        let (bids, asks) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
        OrderBook {
            exchange: ExchangeId::BitgetFutures,
            symbol: snap.symbol.clone(),
            bids,
            asks,
            timestamp_ms: ts,
            sequence: Some(state.book.last_update_id),
        }
    } else {
        state.book.to_orderbook(
            ExchangeId::BitgetFutures,
            snap.symbol.clone(),
            ts,
            TOP_LEVELS,
        )
    };
    let _ = out_tx.send(ob).await;
}

fn spawn_bootstrap(
    symbol: Symbol,
    rest: Arc<BitgetFuturesRest>,
    snap_tx: mpsc::Sender<SnapshotMsg>,
) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            // REST у Bitget futures capped 100 уровней. Используется только
            // на re-sync. Глубина дофиксится последующими WS update'ами.
            match rest.orderbook(&symbol, 100).await {
                Ok(ob) => {
                    let bids: Vec<(Decimal, Decimal)> =
                        ob.bids.iter().map(|l| (l.price, l.qty)).collect();
                    let asks: Vec<(Decimal, Decimal)> =
                        ob.asks.iter().map(|l| (l.price, l.qty)).collect();
                    let last_seq = ob.sequence.unwrap_or_else(|| ob.timestamp_ms);
                    let _ = snap_tx
                        .send(SnapshotMsg {
                            symbol,
                            last_seq,
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
                        "Bitget Futures REST snapshot failed"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}
