use crate::futures::ct_vals;
use crate::futures::mapper::*;
use crate::futures::rest::OkxSwapRest;
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

const WS_PUBLIC_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const WS_BUSINESS_URL: &str = "wss://ws.okx.com:8443/ws/v5/business";
const EXCHANGE: ExchangeId = ExchangeId::Okx;

/// См. spot/ws.rs — канал `books` отдаёт 400 уровней.
const TOP_LEVELS: usize = 400;
const MAX_BUFFER: usize = 1024;
/// См. spot/ws.rs::MAX_ARGS_PER_CONNECTION — те же причины (эмпирически).
const MAX_ARGS_PER_CONNECTION: usize = 40;
const SHARD_STAGGER_MS: u64 = 500;

// ---------------------------------------------------------------------------
// Core helper — same pattern as spot ws
// ---------------------------------------------------------------------------

async fn subscribe_and_stream(
    url: &str,
    args: Vec<serde_json::Value>,
) -> Result<BoxStream<serde_json::Value>> {
    let url_owned = url.to_string();

    let (ws_stream, _) = connect_async(&url_owned)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
    write
        .send(Message::text(sub.to_string()))
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(2048);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            let _ = write.send(Message::text("ping".to_string())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
            ping_interval.tick().await;

            // См. spot/ws.rs — periodic re-subscribe для refresh stale levels.
            let mut resub_interval = tokio::time::interval(Duration::from_secs(300));
            resub_interval.tick().await;

            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text("ping".to_string())).await.is_err() {
                            break;
                        }
                    }
                    _ = resub_interval.tick() => {
                        debug!("OKX futures WS: periodic resubscribe to refresh stale levels");
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
                                    if json.get("event").is_some() {
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
                                warn!("OKX futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("OKX futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("OKX futures WS stream ended unexpectedly");
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
                warn!("OKX futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(&url_owned).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
                        if new_write
                            .send(Message::text(sub.to_string()))
                            .await
                            .is_err()
                        {
                            warn!("OKX futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("OKX futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("OKX futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("OKX futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

fn sub_arg(channel: &str, inst_id: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        "instId": inst_id
    })
}

fn sub_arg_inst_type(channel: &str, inst_type: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        "instType": inst_type
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
    let inst_id = unified_to_okx_swap(symbol);
    let arg = sub_arg("trades", &inst_id);
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<OkxWsTradeData> =
                    serde_json::from_value(data.clone()).ok()?;
                if !trades.is_empty() {
                    let converted: Vec<Trade> = trades
                        .into_iter()
                        .map(|t| t.into_trade(EXCHANGE))
                        .collect();
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
    let inst_id = unified_to_okx_swap(symbol);
    let channel = interval_to_okx_ws(interval);
    let arg = sub_arg(&channel, &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(WS_BUSINESS_URL, vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?.as_array()?;
            let row: Vec<String> = first
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            parse_kline_row(&row, EXCHANGE, &sym, interval)
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
    let inst_id = unified_to_okx_swap(symbol);
    let arg = sub_arg("mark-price", &inst_id);
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: OkxWsMarkPriceData = serde_json::from_value(first.clone()).ok()?;
        Some(raw.into_mark_price(EXCHANGE))
    })))
}

pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let arg = sub_arg_inst_type("liquidation-orders", "SWAP");
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(
        futures::stream::unfold((raw_stream, sym), |(mut stream, sym)| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let orders: Vec<OkxWsLiquidationData> =
                    serde_json::from_value(data.clone()).ok()?;
                let liqs: Vec<Liquidation> = orders
                    .into_iter()
                    .filter(|o| {
                        okx_inst_id_to_unified(&o.inst_id)
                            .map(|s| s == sym)
                            .unwrap_or(false)
                    })
                    .flat_map(|o| o.into_liquidations(EXCHANGE))
                    .collect();
                if !liqs.is_empty() {
                    return Some((futures::stream::iter(liqs), (stream, sym)));
                }
            }
        })
        .flatten(),
    ))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) orderbook stream — с поддержкой LocalOrderBook
// ---------------------------------------------------------------------------

/// См. spot/ws.rs::stream_orderbooks_batch — та же логика, только
/// `instId` биржи это `{BASE}-{QUOTE}-SWAP`, REST использует `OkxSwapRest`.
pub async fn stream_orderbooks_batch(
    config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let all_args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("books", &unified_to_okx_swap(s)))
        .collect();

    let num_connections = all_args.chunks(MAX_ARGS_PER_CONNECTION).len();
    if num_connections > 1 {
        info!(
            "OKX Futures WS: sharding {} books streams across {} connections",
            all_args.len(),
            num_connections
        );
    }

    // Прогреваем кэш ctVal до старта maintain'а — чтобы первый snapshot
    // уже мог корректно домножать qty на ctVal. Если REST упадёт,
    // ct_vals::ct_val_for будет возвращать 1 (т.е. без множителя).
    ct_vals::warm_up().await;

    let rest = Arc::new(OkxSwapRest::new(config));
    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    for (idx, chunk) in all_args.chunks(MAX_ARGS_PER_CONNECTION).enumerate() {
        if idx > 0 {
            tokio::time::sleep(Duration::from_millis(SHARD_STAGGER_MS)).await;
        }
        let raw = subscribe_and_stream(WS_PUBLIC_URL, chunk.to_vec()).await?;
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
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("trades", &unified_to_okx_swap(s)))
        .collect();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, args).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<OkxWsTradeData> =
                    serde_json::from_value(data.clone()).ok()?;
                if !trades.is_empty() {
                    let converted: Vec<Trade> = trades
                        .into_iter()
                        .map(|t| t.into_trade(EXCHANGE))
                        .collect();
                    return Some((futures::stream::iter(converted), stream));
                }
            }
        })
        .flatten(),
    ))
}

// ---------------------------------------------------------------------------
// Maintain logic (same shape as spot/ws.rs::maintain_shard)
// ---------------------------------------------------------------------------

struct BufferedUpdate {
    seq_id: u64,
    prev_seq_id: i64,
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
    rest: Arc<OkxSwapRest>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SnapshotMsg>(256);

    loop {
        tokio::select! {
            biased;

            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("OKX Futures maintain shard: ws stream ended");
                    return;
                };
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &rest, &snap_tx).await {
                    debug!(error = %e, "okx futures ws event ignored");
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
    rest: &Arc<OkxSwapRest>,
    snap_tx: &mpsc::Sender<SnapshotMsg>,
) -> std::result::Result<(), &'static str> {
    let arg = json.get("arg").ok_or("no arg")?;
    let inst_id = arg
        .get("instId")
        .and_then(|v| v.as_str())
        .ok_or("no instId")?;
    let symbol = okx_inst_id_to_unified(inst_id).ok_or("bad instId")?;

    let action = json
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let data_arr = json
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or("no data array")?;
    let first = data_arr.first().ok_or("empty data")?;
    let book_data: OkxWsBookData =
        serde_json::from_value(first.clone()).map_err(|_| "parse book data")?;

    // OKX SWAP отдаёт sz в числе контрактов. Конвертируем в base-currency
    // qty домножением на ctVal (1 контракт BTC-USDT-SWAP = 0.01 BTC).
    let ct_val = ct_vals::ct_val_for(&symbol).await;
    let bids: Vec<(Decimal, Decimal)> = book_data
        .bids
        .iter()
        .filter_map(|p| parse_level(p).map(|(px, qty)| (px, qty * ct_val)))
        .collect();
    let asks: Vec<(Decimal, Decimal)> = book_data
        .asks
        .iter()
        .filter_map(|p| parse_level(p).map(|(px, qty)| (px, qty * ct_val)))
        .collect();
    let event_time: u64 = book_data.ts.parse().unwrap_or(0);
    let seq_id = book_data.seq_id.ok_or("no seqId")?;
    let prev_seq_id = book_data.prev_seq_id.unwrap_or(-1);

    let state = states.entry(symbol.clone()).or_insert_with(SymbolState::new);

    if action == "snapshot" {
        let mut new_book = LocalOrderBook::with_max_per_side(TOP_LEVELS);
        new_book.set_snapshot(bids.iter().copied(), asks.iter().copied(), seq_id);

        let prev_book = std::mem::take(&mut state.book);
        state.book = new_book;
        state.last_seq = seq_id;
        state.buffer.clear();
        state.bootstrap_in_flight = false;

        info!(
            symbol = %symbol,
            levels_bids = state.book.bids.len(),
            levels_asks = state.book.asks.len(),
            seq = seq_id,
            was_resync = prev_book.last_update_id > 0,
            "OKX Futures: local book initialized"
        );

        let ob = if prev_book.last_update_id > 0 {
            let (b, a) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
            OrderBook {
                exchange: EXCHANGE,
                symbol,
                bids: b,
                asks: a,
                timestamp_ms: event_time,
                sequence: Some(seq_id),
            }
        } else {
            state.book.to_orderbook(EXCHANGE, symbol, event_time, TOP_LEVELS)
        };
        let _ = out_tx.send(ob).await;
        return Ok(());
    }

    if !state.book.ready {
        if state.buffer.len() < MAX_BUFFER {
            state.buffer.push_back(BufferedUpdate {
                seq_id,
                prev_seq_id,
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

    if seq_id <= state.last_seq {
        return Ok(());
    }

    let expected = state.last_seq as i64;
    if prev_seq_id != expected && prev_seq_id != seq_id as i64 {
        state.resync_count += 1;
        warn!(
            symbol = %symbol,
            local_seq = state.last_seq,
            event_prev_seq = prev_seq_id,
            event_seq = seq_id,
            resync_count = state.resync_count,
            "OKX Futures: sequence gap, triggering re-sync"
        );
        state.book.ready = false;
        state.buffer.clear();
        state.buffer.push_back(BufferedUpdate {
            seq_id,
            prev_seq_id,
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

    state.book.apply_diff(bids.iter().copied(), asks.iter().copied(), seq_id);
    state.last_seq = seq_id;

    let ob = OrderBook {
        exchange: EXCHANGE,
        symbol,
        bids: bids.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        asks: asks.iter().map(|(p, q)| Level::new(*p, *q)).collect(),
        timestamp_ms: event_time,
        sequence: Some(seq_id),
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

    let mut bootstrap_failed = false;
    let mut first_applied = false;
    let mut last_event_time = 0u64;
    while let Some(ev) = state.buffer.pop_front() {
        if ev.seq_id <= new_book.last_update_id {
            continue;
        }
        if !first_applied {
            let lub = new_book.last_update_id as i64;
            if ev.prev_seq_id > lub {
                warn!(
                    symbol = %snap.symbol,
                    snap_seq = lub,
                    event_prev_seq = ev.prev_seq_id,
                    event_seq = ev.seq_id,
                    "OKX Futures: snapshot too old vs buffer, retrying"
                );
                bootstrap_failed = true;
                break;
            }
            first_applied = true;
        }
        last_event_time = ev.event_time;
        new_book.apply_diff(
            ev.bids.iter().copied(),
            ev.asks.iter().copied(),
            ev.seq_id,
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
        last_seq = new_book.last_update_id,
        was_resync = prev_book.last_update_id > 0,
        "OKX Futures: REST bootstrap applied"
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
            exchange: EXCHANGE,
            symbol: snap.symbol.clone(),
            bids,
            asks,
            timestamp_ms: ts,
            sequence: Some(state.book.last_update_id),
        }
    } else {
        state
            .book
            .to_orderbook(EXCHANGE, snap.symbol.clone(), ts, TOP_LEVELS)
    };
    let _ = out_tx.send(ob).await;
}

fn spawn_bootstrap(
    symbol: Symbol,
    rest: Arc<OkxSwapRest>,
    snap_tx: mpsc::Sender<SnapshotMsg>,
) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            // REST `OkxSwapRest::orderbook` уже домножает qty на ctVal
            // (см. futures/rest.rs::orderbook), поэтому здесь просто берём
            // bids/asks как есть.
            match rest.orderbook(&symbol, 5000).await {
                Ok(ob) => {
                    let bids: Vec<(Decimal, Decimal)> =
                        ob.bids.iter().map(|l| (l.price, l.qty)).collect();
                    let asks: Vec<(Decimal, Decimal)> =
                        ob.asks.iter().map(|l| (l.price, l.qty)).collect();
                    let Some(seq) = ob.sequence else {
                        warn!(symbol = %symbol, "OKX Futures snapshot missing sequence, retrying");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        continue;
                    };
                    let _ = snap_tx
                        .send(SnapshotMsg {
                            symbol,
                            last_seq: seq,
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
                        "OKX Futures REST snapshot failed"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}

fn parse_level(raw: &[String]) -> Option<(Decimal, Decimal)> {
    if raw.len() < 2 {
        return None;
    }
    let price = Decimal::from_str(&raw[0]).ok()?;
    let qty = Decimal::from_str(&raw[1]).ok()?;
    Some((price, qty))
}
