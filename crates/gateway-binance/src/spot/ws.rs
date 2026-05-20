use crate::local_book::LocalOrderBook;
use crate::rate_limit::spot_limiter;
use crate::spot::mapper::*;
use crate::spot::rest::BinanceRest;
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

/// Сколько уровней отдаём в каждом snapshot/diff emit'е.
/// Binance REST `limit=1000` даёт 1000 уровней, этим и ограничиваем.
const TOP_LEVELS: usize = 1000;

/// Максимальный размер буфера diff'ов на символ во время ожидания snapshot'а.
/// При нормальном rate ~10 событий/сек, 600 событий = ~60 сек ожидания —
/// этого с запасом хватает на любой rate-limited bootstrap.
const MAX_BUFFER: usize = 1024;

/// REST endpoint weight за вызов `/api/v3/depth?limit=1000`.
const DEPTH_WEIGHT: u32 = 20;

const WS_URL: &str = "wss://stream.binance.com:9443/ws";

/// Maximum number of streams per single WebSocket connection.
/// Binance allows up to 1024 for spot but we use a lower limit
/// for better stability and load distribution.
const MAX_STREAMS_PER_CONNECTION: usize = 100;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to a Binance WebSocket endpoint, optionally send a SUBSCRIBE message,
/// and return a [`BoxStream`] that yields parsed JSON values.
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

    let (ws_stream, _) = connect_async(&url).await.map_err(|e| GatewayError::WebSocket {
        exchange: ExchangeId::BinanceSpot,
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
                exchange: ExchangeId::BinanceSpot,
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
                                    warn!("Binance WS pong send failed");
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Binance WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Binance WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Binance WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = ping_interval.tick() => {
                        if write.send(Message::Ping(vec![].into())).await.is_err() {
                            warn!("Binance WS ping send failed");
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
                warn!("Binance WS reconnecting in {backoff:?}…");
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
                                warn!("Binance WS subscribe failed after reconnect");
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                continue;
                            }
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        ping_interval.reset();
                        info!("Binance WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Binance WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Binance WS stream ended");
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
        let raw: BinanceWsDepthRaw = serde_json::from_value(json).ok()?;
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
        let raw: BinanceWsTradeRaw = serde_json::from_value(json).ok()?;
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
        let raw: BinanceWsKlineMsg = serde_json::from_value(json).ok()?;
        Some(raw.into_candle())
    })))
}

// ---------------------------------------------------------------------------
// Combined (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream consistent top-1000 order-book updates for multiple symbols.
///
/// Поддерживает локальный стакан per-symbol по стандартной схеме Binance:
/// 1. Подписывается на `@depth@100ms` (diff-стрим).
/// 2. На первый diff каждого символа — отправляет background REST GET
///    `/api/v3/depth?limit=1000` через глобальный rate limiter.
/// 3. Буферизует входящие diff'ы до получения snapshot'а.
/// 4. Валидирует sequence (Spot: `U <= lastUpdateId+1 <= u`).
/// 5. После initial sync отдаёт полный snapshot top-1000 как `OrderBook`.
/// 6. Каждый последующий diff отдаётся как обычный `OrderBook` с пришедшими
///    уровнями (qty=0 для удалений). Эти уровни уже из консистентной книги.
/// 7. На разрывах sequence — re-sync: очистка локальной книги и новый REST
///    snapshot. После re-sync отдаётся diff между старой и новой книгой
///    (удалённые уровни как qty=0).
///
/// Автоматически шардит подписки на несколько WS-соединений
/// (`MAX_STREAMS_PER_CONNECTION` потоков на соединение).
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
            "Binance Spot WS: sharding {} depth streams across {} connections",
            all_streams.len(),
            num_connections
        );
    }

    let rest = Arc::new(BinanceRest::new(config));
    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(WS_URL, chunk.to_vec()).await?;
        let shard_tx = out_tx.clone();
        let shard_rest = rest.clone();
        tokio::spawn(maintain_shard(raw, shard_tx, shard_rest));
    }

    Ok(Box::pin(ReceiverStream::new(out_rx)))
}

// ---------------------------------------------------------------------------
// Maintain logic
// ---------------------------------------------------------------------------

/// Одно diff-событие, буферизованное во время ожидания REST snapshot'а.
struct BufferedDiff {
    first_update_id: u64,
    last_update_id: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
    event_time: u64,
}

/// Состояние одного символа в shard'е.
struct SymbolState {
    book: LocalOrderBook,
    /// События, накопленные пока ждём REST snapshot.
    buffer: VecDeque<BufferedDiff>,
    /// REST snapshot bootstrap уже запущен/был запущен для текущего цикла.
    bootstrap_in_flight: bool,
    /// Сколько раз подряд была re-sync (для exponential backoff log spam).
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

/// Результат REST snapshot'а, доставленный maintain task'у.
struct SnapshotMsg {
    symbol: Symbol,
    last_update_id: u64,
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
}

/// Maintain loop одного WS-shard'а.
/// Читает raw JSON events из WS, диспетчит REST snapshot'ы, эмитит OrderBook'и.
async fn maintain_shard(
    mut raw: BoxStream<serde_json::Value>,
    out_tx: mpsc::Sender<OrderBook>,
    rest: Arc<BinanceRest>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SnapshotMsg>(256);

    loop {
        tokio::select! {
            biased;

            // 1. WS diff event.
            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("Binance Spot maintain shard: ws stream ended");
                    return;
                };
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &rest, &snap_tx).await {
                    debug!(error = %e, "binance spot ws event ignored");
                }
            }

            // 2. REST snapshot completed.
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
    rest: &Arc<BinanceRest>,
    snap_tx: &mpsc::Sender<SnapshotMsg>,
) -> std::result::Result<(), &'static str> {
    // Combined-stream обёртка: { "stream": "...", "data": {...} }.
    let depth_json = json.get("data").unwrap_or(json);
    let raw: BinanceWsDepthRaw = serde_json::from_value(depth_json.clone())
        .map_err(|_| "parse depth")?;

    let symbol = binance_symbol_to_unified(&raw.symbol);
    let bids: Vec<(Decimal, Decimal)> = raw
        .bids
        .iter()
        .filter_map(|p| parse_level(p))
        .collect();
    let asks: Vec<(Decimal, Decimal)> = raw
        .asks
        .iter()
        .filter_map(|p| parse_level(p))
        .collect();

    let state = states.entry(symbol.clone()).or_insert_with(SymbolState::new);

    if !state.book.ready {
        // Накапливаем буфер до прихода snapshot'а.
        if state.buffer.len() < MAX_BUFFER {
            state.buffer.push_back(BufferedDiff {
                first_update_id: raw.first_update_id,
                last_update_id: raw.last_update_id,
                bids,
                asks,
                event_time: raw.event_time,
            });
        }
        // Если bootstrap не запущен — запускаем.
        if !state.bootstrap_in_flight {
            state.bootstrap_in_flight = true;
            spawn_bootstrap(symbol, rest.clone(), snap_tx.clone());
        }
        return Ok(());
    }

    // Книга готова — применяем diff с sequence validation.
    let lub = state.book.last_update_id;
    // Устаревшее событие (u <= lub) — молча дропаем. Такое случается,
    // когда WS event'ы отправлены биржей до момента REST snapshot'а, но
    // доставлены TCP-сокетом уже после receive'а snapshot'а.
    if raw.last_update_id <= lub {
        return Ok(());
    }
    let valid = raw.first_update_id <= lub + 1 && raw.last_update_id >= lub + 1;
    if !valid {
        // Gap: re-sync. Сохраняем старую книгу для diff'а,
        // запускаем bootstrap, дальше события буферизуем.
        state.resync_count += 1;
        warn!(
            symbol = %symbol,
            local_lub = lub,
            event_U = raw.first_update_id,
            event_u = raw.last_update_id,
            resync_count = state.resync_count,
            "Binance Spot: sequence gap, triggering re-sync"
        );
        // Очищаем книгу но запоминаем предыдущее состояние внутри bootstrap'а
        // через old book ниже. Здесь же — сохраняем "prev_for_diff" в State.
        state.book.ready = false;
        state.buffer.clear();
        state.buffer.push_back(BufferedDiff {
            first_update_id: raw.first_update_id,
            last_update_id: raw.last_update_id,
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

    // Применяем diff к локальной книге.
    state.book.apply_diff(
        bids.iter().copied(),
        asks.iter().copied(),
        raw.last_update_id,
    );

    // Emit raw diff в выходной стрим.
    let ob = OrderBook {
        exchange: ExchangeId::BinanceSpot,
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

    // Sохраняем prev book для diff'а (если был re-sync).
    let prev_book = if state.book.ready {
        // Не должно случаться, но если ready=true и пришёл snapshot — игнор.
        return;
    } else {
        std::mem::take(&mut state.book)
    };

    // Устанавливаем новый snapshot.
    let mut new_book = LocalOrderBook::new();
    new_book.set_snapshot(
        snap.bids.iter().copied(),
        snap.asks.iter().copied(),
        snap.last_update_id,
    );

    // Дренируем буфер с правилом Spot:
    // Найти первый event где U <= snapshot.lastUpdateId+1 AND u >= snapshot.lastUpdateId+1.
    // Все события с u < snapshot.lastUpdateId — выбрасываем как устаревшие.
    let mut bootstrap_failed = false;
    let mut first_applied = false;
    let mut last_event_time = 0u64;
    while let Some(ev) = state.buffer.pop_front() {
        if ev.last_update_id < new_book.last_update_id {
            // Устаревший event, дропаем.
            continue;
        }
        if !first_applied {
            // Валидируем первый сохранившийся event.
            let lub = new_book.last_update_id;
            if !(ev.first_update_id <= lub + 1 && ev.last_update_id >= lub + 1) {
                // Snapshot слишком старый для буфера — повторяем bootstrap.
                warn!(
                    symbol = %snap.symbol,
                    snap_lub = lub,
                    event_U = ev.first_update_id,
                    event_u = ev.last_update_id,
                    "Binance Spot: snapshot too old vs buffer, retrying"
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
            ev.last_update_id,
        );
    }

    if bootstrap_failed {
        // Откатываем — оставляем книгу не-ready, перезапускаем bootstrap
        // на следующем WS event. resync_count++ уже в re-sync ветке.
        state.book = prev_book; // вернём prev (он не-ready)
        state.buffer.clear();
        return;
    }

    info!(
        symbol = %snap.symbol,
        levels_bids = new_book.bids.len(),
        levels_asks = new_book.asks.len(),
        last_update_id = new_book.last_update_id,
        was_resync = prev_book.last_update_id > 0,
        "Binance Spot: local book initialized"
    );
    state.book = new_book;
    state.resync_count = 0;

    // Emit полный snapshot/diff.
    let ts = if last_event_time > 0 { last_event_time } else { 0 };
    let ob = if prev_book.last_update_id > 0 {
        // Re-sync: emit diff между prev и new (включая qty=0 для удалённых).
        let (bids, asks) = state.book.diff_against_prev(&prev_book, TOP_LEVELS);
        OrderBook {
            exchange: ExchangeId::BinanceSpot,
            symbol: snap.symbol.clone(),
            bids,
            asks,
            timestamp_ms: ts,
            sequence: Some(state.book.last_update_id),
        }
    } else {
        // Initial: emit полный top-1000.
        state.book.to_orderbook(
            ExchangeId::BinanceSpot,
            snap.symbol.clone(),
            ts,
            TOP_LEVELS,
        )
    };
    let _ = out_tx.send(ob).await;
}

/// Запуск background task'а для REST snapshot'а одного символа.
/// Использует глобальный rate limiter, retry на ошибки.
fn spawn_bootstrap(
    symbol: Symbol,
    rest: Arc<BinanceRest>,
    snap_tx: mpsc::Sender<SnapshotMsg>,
) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            spot_limiter().acquire(DEPTH_WEIGHT).await;
            match rest.orderbook(&symbol, 1000).await {
                Ok(ob) => {
                    let bids: Vec<(Decimal, Decimal)> =
                        ob.bids.iter().map(|l| (l.price, l.qty)).collect();
                    let asks: Vec<(Decimal, Decimal)> =
                        ob.asks.iter().map(|l| (l.price, l.qty)).collect();
                    let Some(seq) = ob.sequence else {
                        warn!(symbol = %symbol, "Binance Spot snapshot missing sequence, retrying");
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
                    warn!(symbol = %symbol, error = %e, backoff_ms = backoff.as_millis() as u64, "Binance Spot REST snapshot failed");
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
            "Binance Spot WS: sharding {} trade streams across {} connections",
            all_streams.len(),
            num_connections
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(WS_URL, chunk.to_vec()).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.filter_map(|json| async move {
            let trade_json = json.get("data").cloned().unwrap_or(json);
            let raw: BinanceWsTradeRaw = serde_json::from_value(trade_json).ok()?;
            Some(raw.into_trade())
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}
