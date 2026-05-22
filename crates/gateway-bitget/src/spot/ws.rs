use crate::spot::mapper::*;
use crate::spot::rest::BitgetRest;
use futures::{SinkExt, StreamExt};
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

/// Канал `books` Bitget'а на снапшоте отдаёт 500 уровней, на update'ах — дельты.
/// Эмитим всю поддерживаемую глубину.
const TOP_LEVELS: usize = 500;

/// Сколько update'ов копим в буфер пока ждём REST snapshot на re-sync.
const MAX_BUFFER: usize = 1024;

/// Bitget публикует ограничение ~50 подписок на одно соединение. Держим
/// консервативный 40 чтобы не упереться при добавлении символов.
const MAX_ARGS_PER_CONNECTION: usize = 40;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

async fn subscribe_and_stream(
    args: Vec<serde_json::Value>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BitgetSpot,
                message: e.to_string(),
            })?;

    let (mut write, read) = ws_stream.split();

    let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
    write
        .send(Message::text(sub.to_string()))
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: ExchangeId::BitgetSpot,
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

            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text("ping".to_string())).await.is_err() {
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
                                        debug!("Bitget WS subscribed: {}", text);
                                        continue;
                                    }
                                    if json.get("event").and_then(|v| v.as_str()) == Some("error") {
                                        warn!("Bitget WS error event: {}", text);
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
                                warn!("Bitget WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Bitget WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Bitget WS stream ended unexpectedly");
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
                warn!("Bitget WS reconnecting in {backoff:?}…");
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
                            warn!("Bitget WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Bitget WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Bitget WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Bitget WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

fn sub_arg(channel: &str, inst_id: &str) -> serde_json::Value {
    serde_json::json!({
        "instType": "SPOT",
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

pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("trade", &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

    Ok(Box::pin(
        futures::stream::unfold((raw_stream, sym), |(mut stream, sym)| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                if let Ok(trades) = serde_json::from_value::<Vec<BitgetWsTradeRaw>>(data.clone()) {
                    if !trades.is_empty() {
                        let converted: Vec<Trade> =
                            trades.into_iter().map(|t| t.into_trade(sym.clone())).collect();
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
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

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
// Batch (multi-symbol) streams — с поддержкой LocalOrderBook
// ---------------------------------------------------------------------------

/// Стрим консистентного стакана (top-500) для множества символов через канал
/// `books`. Поддерживает локальный стакан per-symbol по схеме Bitget:
///
/// 1. После subscribe канал шлёт `action="snapshot"` (500 уровней) → reset.
/// 2. Дальнейшие `action="update"` валидируются: `pseq == local.last_seq`.
/// 3. На gap → REST snapshot через `/api/v2/spot/market/orderbook` (max 150)
///    с буферизацией update'ов до его прихода.
/// 4. На initial sync эмитим top-500.
/// 5. Каждый update эмитится как `OrderBook` с пришедшими уровнями.
/// 6. На re-sync — diff между старой и новой книгой.
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
            "Bitget Spot WS: sharding {} books streams across {} connections",
            all_args.len(),
            num_connections
        );
    }

    let rest = Arc::new(BitgetRest::new(config));
    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    for chunk in all_args.chunks(MAX_ARGS_PER_CONNECTION) {
        let raw = subscribe_and_stream(chunk.to_vec()).await?;
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
    use futures::stream::SelectAll;
    const CHUNK_SIZE: usize = 50;

    let mut all = SelectAll::new();
    for chunk in symbols.chunks(CHUNK_SIZE) {
        let args: Vec<serde_json::Value> = chunk
            .iter()
            .map(|s| sub_arg("trade", &unified_to_bitget(s)))
            .collect();
        let raw_stream = subscribe_and_stream(args).await?;
        let chunk_stream: BoxStream<Trade> = Box::pin(
            futures::stream::unfold(raw_stream, |mut stream| async move {
                loop {
                    let json = stream.next().await?;
                    let arg = json.get("arg")?;
                    let inst_id = arg.get("instId")?.as_str()?;
                    let symbol = bitget_symbol_to_unified(inst_id);
                    let data = json.get("data")?;
                    if let Ok(trades) =
                        serde_json::from_value::<Vec<BitgetWsTradeRaw>>(data.clone())
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
            book: LocalOrderBook::new(),
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
    rest: Arc<BitgetRest>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SnapshotMsg>(256);

    loop {
        tokio::select! {
            biased;

            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("Bitget Spot maintain shard: ws stream ended");
                    return;
                };
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &rest, &snap_tx).await {
                    debug!(error = %e, "bitget spot ws event ignored");
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
    rest: &Arc<BitgetRest>,
    snap_tx: &mpsc::Sender<SnapshotMsg>,
) -> std::result::Result<(), &'static str> {
    let arg = json.get("arg").ok_or("no arg")?;
    let inst_id = arg
        .get("instId")
        .and_then(|v| v.as_str())
        .ok_or("no instId")?;
    let symbol = bitget_symbol_to_unified(inst_id);

    let action = json
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let data_arr = json
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or("no data array")?;
    let first = data_arr.first().ok_or("empty data")?;
    let book_data: BitgetWsOrderBook =
        serde_json::from_value(first.clone()).map_err(|_| "parse book data")?;

    let bids = parse_book_levels(&book_data.bids);
    let asks = parse_book_levels(&book_data.asks);
    let event_time: u64 = book_data.ts.parse().unwrap_or(0);
    let seq = book_data.seq.ok_or("no seq")?;
    let pseq = book_data.pseq.unwrap_or(0);

    let state = states.entry(symbol.clone()).or_insert_with(SymbolState::new);

    if action == "snapshot" {
        let mut new_book = LocalOrderBook::new();
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
            "Bitget Spot: local book initialized"
        );

        let ob = if prev_book.last_update_id > 0 {
            let (b, a) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
            OrderBook {
                exchange: ExchangeId::BitgetSpot,
                symbol,
                bids: b,
                asks: a,
                timestamp_ms: event_time,
                sequence: Some(seq),
            }
        } else {
            state
                .book
                .to_orderbook(ExchangeId::BitgetSpot, symbol, event_time, TOP_LEVELS)
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

    // Bitget: для update pseq должен быть равен последнему seq.
    if pseq != state.last_seq {
        state.resync_count += 1;
        warn!(
            symbol = %symbol,
            local_seq = state.last_seq,
            event_pseq = pseq,
            event_seq = seq,
            resync_count = state.resync_count,
            "Bitget Spot: sequence gap, triggering re-sync"
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

    state.book.apply_diff(bids.iter().copied(), asks.iter().copied(), seq);
    state.last_seq = seq;

    let ob = OrderBook {
        exchange: ExchangeId::BitgetSpot,
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
    let mut new_book = LocalOrderBook::new();
    new_book.set_snapshot(
        snap.bids.iter().copied(),
        snap.asks.iter().copied(),
        snap.last_seq,
    );

    // REST snapshot Bitget'а не несёт seq (only ts), поэтому мы кладём в
    // last_update_id значение из WS-update'а, который инициировал re-sync.
    // Дренируем буфер: все события с seq <= snap.last_seq устаревшие.
    let mut last_event_time = 0u64;
    while let Some(ev) = state.buffer.pop_front() {
        if ev.seq <= new_book.last_update_id {
            continue;
        }
        last_event_time = ev.event_time;
        new_book.apply_diff(
            ev.bids.iter().copied(),
            ev.asks.iter().copied(),
            ev.seq,
        );
    }

    info!(
        symbol = %snap.symbol,
        levels_bids = new_book.bids.len(),
        levels_asks = new_book.asks.len(),
        last_seq = new_book.last_update_id,
        was_resync = prev_book.last_update_id > 0,
        "Bitget Spot: REST bootstrap applied"
    );
    state.last_seq = new_book.last_update_id;
    state.book = new_book;
    state.resync_count = 0;

    let ts = if last_event_time > 0 { last_event_time } else { 0 };
    let ob = if prev_book.last_update_id > 0 {
        let (bids, asks) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
        OrderBook {
            exchange: ExchangeId::BitgetSpot,
            symbol: snap.symbol.clone(),
            bids,
            asks,
            timestamp_ms: ts,
            sequence: Some(state.book.last_update_id),
        }
    } else {
        state
            .book
            .to_orderbook(ExchangeId::BitgetSpot, snap.symbol.clone(), ts, TOP_LEVELS)
    };
    let _ = out_tx.send(ob).await;
}

fn spawn_bootstrap(
    symbol: Symbol,
    rest: Arc<BitgetRest>,
    snap_tx: mpsc::Sender<SnapshotMsg>,
) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            // REST у Bitget'а capped 150 уровней. Используется только на
            // re-sync (snapshot обычно приходит сразу из WS). Глубину
            // дофиксит первый же WS update.
            match rest.orderbook(&symbol, 150).await {
                Ok(ob) => {
                    let bids: Vec<(Decimal, Decimal)> =
                        ob.bids.iter().map(|l| (l.price, l.qty)).collect();
                    let asks: Vec<(Decimal, Decimal)> =
                        ob.asks.iter().map(|l| (l.price, l.qty)).collect();
                    // REST у Bitget'а seq не возвращает. Используем ts как
                    // monotonic id для last_update_id — это не идеально, но
                    // дренаж буфера всё равно отбрасывает по локальному seq
                    // следующего WS update'а.
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
                        "Bitget Spot REST snapshot failed"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}
