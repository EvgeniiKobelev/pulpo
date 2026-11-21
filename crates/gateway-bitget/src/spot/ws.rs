use crate::spot::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://ws.bitget.com/v2/ws/public";

/// Канал `books` Bitget'а на снапшоте отдаёт 500 уровней, на update'ах — дельты.
/// Эмитим всю поддерживаемую глубину.
const TOP_LEVELS: usize = 500;

/// Bitget публикует ограничение ~50 подписок на одно соединение. Держим
/// консервативный 40 чтобы не упереться при добавлении символов.
const MAX_ARGS_PER_CONNECTION: usize = 40;

/// Сколько ждём WS-снапшот после переподписки на разрыве seq, прежде чем
/// переподписаться снова; дальше ожидание удваивается до `RESUB_BACKOFF_MAX`.
/// Снапшот приходит за доли секунды — 10 с означают потерянный subscribe.
const RESUB_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const RESUB_BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Как часто шард проверяет символы, ждущие снапшот: тихий символ без дельт
/// иначе не дал бы повода повторить потерянную переподписку.
const RESUB_CHECK_INTERVAL: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// `periodic_resub` — принудительный re-subscribe каждые 5 мин. Нужен ТОЛЬКО
/// каналу `books` (освежает stale-уровни, см. комментарий у resub_interval).
/// На каналах `trade` Bitget в ответ на каждый subscribe шлёт снапшот
/// последних трейдов (`action="snapshot"`) — на неликвидах это одни и те же
/// трейды часами → дубли у потребителей (инцидент: ×1.1–1.6 строк в
/// ClickHouse-истории, до 283 копий одного принта).
async fn subscribe_and_stream(
    args: Vec<serde_json::Value>,
    periodic_resub: bool,
) -> Result<BoxStream<serde_json::Value>> {
    subscribe_and_stream_inner(args, periodic_resub, None).await
}

/// Поток `books` с каналом переподписки отдельных символов: шард шлёт туда
/// arg символа, у которого разошёлся seq, и Bitget отвечает свежим
/// WS-снапшотом с настоящим seq — цепочка дельт продолжается от него.
async fn subscribe_books(
    args: Vec<serde_json::Value>,
) -> Result<(
    BoxStream<serde_json::Value>,
    mpsc::UnboundedSender<serde_json::Value>,
)> {
    let (resub_tx, resub_rx) = mpsc::unbounded_channel();
    let stream = subscribe_and_stream_inner(args, true, Some(resub_rx)).await?;
    Ok((stream, resub_tx))
}

/// Следующий arg на переподписку; без канала — никогда.
async fn next_resub(
    rx: &mut Option<mpsc::UnboundedReceiver<serde_json::Value>>,
) -> Option<serde_json::Value> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn subscribe_and_stream_inner(
    args: Vec<serde_json::Value>,
    periodic_resub: bool,
    mut resub_rx: Option<mpsc::UnboundedReceiver<serde_json::Value>>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) = connect_timeout(connect_async(WS_URL))
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
        let mut idle = ReadIdle::new(WS_READ_IDLE_TIMEOUT);

        'outer: loop {
            let _ = write.send(Message::text("ping".to_string())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
            ping_interval.tick().await;

            // Bitget WS `books` отдаёт snapshot 200 уровней + дельты, но
            // дельты приходят только для top-N где N меньше 200. Уровни за
            // пределами этого N остаются stale в локальной книге pulpo
            // навсегда (биржа их обновления просто не шлёт). Каждые 5 мин
            // принудительно перезаписываемся — Bitget отвечает свежим
            // snapshot, и stale-уровни в LocalOrderBook обновляются
            // через diff_against_prev на pulpo'шной стороне.
            let mut resub_interval = tokio::time::interval(Duration::from_secs(300));
            resub_interval.tick().await; // первый тик мгновенный, скипаем
            idle.touch();

            loop {
                // biased: тикеры раньше чтения — поток данных их не вытеснит;
                // дедлайн тишины последним — при готовом кадре он не выигрывает.
                tokio::select! {
                    biased;
                    // Потребитель бросил стрим — закрываем соединение сразу.
                    _ = tx.closed() => break 'outer,
                    _ = ping_interval.tick() => {
                        if write.send(Message::text("ping".to_string())).await.is_err() {
                            break;
                        }
                    }
                    _ = resub_interval.tick(), if periodic_resub => {
                        debug!("Bitget spot WS: periodic resubscribe to refresh stale levels");
                        let sub = serde_json::json!({"op":"subscribe","args":args.clone()});
                        if write.send(Message::text(sub.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Some(arg) = next_resub(&mut resub_rx) => {
                        let sub = serde_json::json!({"op": "subscribe", "args": [arg]});
                        if write.send(Message::text(sub.to_string())).await.is_err() {
                            break;
                        }
                    }
                    msg = read.next() => {
                        idle.touch();
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
                    _ = idle.expired() => {
                        warn!(
                            args = args.len(),
                            idle_secs = idle.timeout().as_secs(),
                            "Bitget Spot WS: no frames, connection presumed dead — reconnecting"
                        );
                        break;
                    }
                }
            }

            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Bitget WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_timeout(connect_async(WS_URL)).await {
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

pub async fn stream_trades(_config: &ExchangeConfig, symbol: &Symbol) -> Result<BoxStream<Trade>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("trade", &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![arg], false).await?;

    Ok(Box::pin(
        futures::stream::unfold((raw_stream, sym), |(mut stream, sym)| async move {
            loop {
                let json = stream.next().await?;
                // `action="snapshot"` — повтор последних трейдов при (пере)подписке,
                // не live-поток: эмитить нельзя, иначе дубли.
                if json.get("action").and_then(|v| v.as_str()) == Some("snapshot") {
                    continue;
                }
                let data = json.get("data")?;
                if let Ok(trades) = serde_json::from_value::<Vec<BitgetWsTradeRaw>>(data.clone()) {
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
// Batch (multi-symbol) streams — с поддержкой LocalOrderBook
// ---------------------------------------------------------------------------

/// Стрим консистентного стакана (top-500) для множества символов через канал
/// `books`. Поддерживает локальный стакан per-symbol по схеме Bitget:
///
/// 1. После subscribe канал шлёт `action="snapshot"` (500 уровней) → reset.
/// 2. Дальнейшие `action="update"` валидируются: `pseq == local.last_seq`.
/// 3. На gap → переподписка этого instId на том же соединении: Bitget
///    отвечает WS-снапшотом с seq; дельты до него не копим.
/// 4. На initial sync эмитим top-500.
/// 5. Каждый update эмитится как `OrderBook` с пришедшими уровнями.
/// 6. На re-sync — diff между старой и новой книгой.
pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
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

    let (out_tx, out_rx) = mpsc::channel::<OrderBook>(8192);

    for chunk in all_args.chunks(MAX_ARGS_PER_CONNECTION) {
        let (raw, resub_tx) = subscribe_books(chunk.to_vec()).await?;
        let shard_tx = out_tx.clone();
        tokio::spawn(maintain_shard(raw, shard_tx, resub_tx));
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

struct SymbolState {
    book: LocalOrderBook,
    last_seq: u64,
    /// Переподписка на разрыве seq ждёт WS-снапшот.
    resub: Option<PendingResub>,
    resync_count: u32,
}

/// Отправленная переподписка символа: когда ушла и через сколько повторить.
struct PendingResub {
    sent_at: Instant,
    retry_after: Duration,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            book: LocalOrderBook::with_max_per_side(TOP_LEVELS),
            last_seq: 0,
            resub: None,
            resync_count: 0,
        }
    }
}

/// Книга символа разошлась с биржей (разрыв seq или дельта до снапшота):
/// просим у Bitget свежий WS-снапшот переподпиской одного instId.
///
/// REST-снапшот Bitget сюда не годится: в нём нет seq. Подставленный вместо
/// seq `timestamp_ms` (~1.8e12) всегда больше WS-seq, и все следующие дельты
/// отбрасывались как старые — книга замерзала до плановой переподписки
/// (до 300 с) и копила фантомы. Плюс REST отдаёт 150 уровней против 200 у WS.
fn request_resub(
    state: &mut SymbolState,
    inst_id: &str,
    resub_tx: &mpsc::UnboundedSender<serde_json::Value>,
    now: Instant,
) {
    if state.resub.is_some() {
        return;
    }
    let _ = resub_tx.send(sub_arg("books", inst_id));
    state.resub = Some(PendingResub {
        sent_at: now,
        retry_after: RESUB_SNAPSHOT_TIMEOUT,
    });
}

/// Повторяет переподписку символам, которые так и не получили снапшот.
fn retry_stale_resubs(
    states: &mut HashMap<Symbol, SymbolState>,
    resub_tx: &mpsc::UnboundedSender<serde_json::Value>,
    now: Instant,
) {
    for (symbol, state) in states.iter_mut() {
        let Some(pending) = state.resub.as_mut() else {
            continue;
        };
        if now.saturating_duration_since(pending.sent_at) < pending.retry_after {
            continue;
        }
        warn!(
            symbol = %symbol,
            waited_secs = pending.retry_after.as_secs(),
            "Bitget Spot: no snapshot after resubscribe, resubscribing again"
        );
        let _ = resub_tx.send(sub_arg("books", &unified_to_bitget(symbol)));
        pending.sent_at = now;
        pending.retry_after = (pending.retry_after * 2).min(RESUB_BACKOFF_MAX);
    }
}

async fn maintain_shard(
    mut raw: BoxStream<serde_json::Value>,
    out_tx: mpsc::Sender<OrderBook>,
    resub_tx: mpsc::UnboundedSender<serde_json::Value>,
) {
    let mut states: HashMap<Symbol, SymbolState> = HashMap::new();
    let mut resub_check = tokio::time::interval(RESUB_CHECK_INTERVAL);
    resub_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            // Выход закрыт (потребитель бросил стрим или combined-подписка
            // не собралась) — шард выходит и дропает `raw`, сырой WS-таск
            // закрывает соединение; иначе шард жил бы вечно с реконнектами.
            _ = out_tx.closed() => {
                debug!("Bitget Spot maintain shard: consumer dropped");
                return;
            }

            ev = raw.next() => {
                let Some(json) = ev else {
                    debug!("Bitget Spot maintain shard: ws stream ended");
                    return;
                };
                let now = Instant::now();
                if let Err(e) = handle_ws_event(&json, &mut states, &out_tx, &resub_tx, now).await {
                    debug!(error = %e, "bitget spot ws event ignored");
                }
            }

            _ = resub_check.tick() => {
                retry_stale_resubs(&mut states, &resub_tx, Instant::now());
            }
        }
    }
}

async fn handle_ws_event(
    json: &serde_json::Value,
    states: &mut HashMap<Symbol, SymbolState>,
    out_tx: &mpsc::Sender<OrderBook>,
    resub_tx: &mpsc::UnboundedSender<serde_json::Value>,
    now: Instant,
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
    let book_data: BitgetWsOrderBook =
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
        if state.resub.take().is_some() {
            state.resync_count = 0;
        }

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

    // Книга не готова (ждём снапшот после переподписки): дельты не копим.
    // Снапшот авторитетен — на одном соединении он приходит после этих дельт
    // и уже включает их; цепочка продолжается от его seq.
    if !state.book.ready {
        request_resub(state, inst_id, resub_tx, now);
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
            "Bitget Spot: sequence gap, resubscribing for a fresh WS snapshot"
        );
        state.book.ready = false;
        request_resub(state, inst_id, resub_tx, now);
        return Ok(());
    }

    state
        .book
        .apply_diff(bids.iter().copied(), asks.iter().copied(), seq);
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

#[cfg(test)]
mod gap_tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    const INST: &str = "HYPEUSDT";
    const S0: u64 = 525_281_364_900;

    fn books(action: &str, seq: u64, pseq: u64, asks: &[(&str, &str)]) -> serde_json::Value {
        let levels: Vec<serde_json::Value> = asks.iter().map(|(p, q)| json!([p, q])).collect();
        json!({
            "action": action,
            "arg": {"instType": "SPOT", "channel": "books", "instId": INST},
            "data": [{
                "bids": [["4.70", "100"]],
                "asks": levels,
                "ts": "1795287201394",
                "seq": seq,
                "pseq": pseq
            }]
        })
    }

    struct Harness {
        states: HashMap<Symbol, SymbolState>,
        out_tx: mpsc::Sender<OrderBook>,
        out_rx: mpsc::Receiver<OrderBook>,
        resub_tx: mpsc::UnboundedSender<serde_json::Value>,
        resub_rx: mpsc::UnboundedReceiver<serde_json::Value>,
        now: Instant,
    }

    impl Harness {
        fn new() -> Self {
            let (out_tx, out_rx) = mpsc::channel(64);
            let (resub_tx, resub_rx) = mpsc::unbounded_channel();
            Self {
                states: HashMap::new(),
                out_tx,
                out_rx,
                resub_tx,
                resub_rx,
                now: Instant::now(),
            }
        }

        async fn feed(&mut self, ev: serde_json::Value) {
            handle_ws_event(&ev, &mut self.states, &self.out_tx, &self.resub_tx, self.now)
                .await
                .expect("books event parses");
        }

        fn emitted(&mut self) -> Vec<OrderBook> {
            std::iter::from_fn(|| self.out_rx.try_recv().ok()).collect()
        }

        fn resubs(&mut self) -> Vec<serde_json::Value> {
            std::iter::from_fn(|| self.resub_rx.try_recv().ok()).collect()
        }

        fn state(&self) -> &SymbolState {
            self.states
                .get(&bitget_symbol_to_unified(INST))
                .expect("symbol state exists")
        }
    }

    /// Регрессия: разрыв seq просит WS-снапшот переподпиской одного instId
    /// (не REST без seq) и до снапшота ничего не эмитит.
    #[tokio::test]
    async fn regression_gap_resubscribes_over_ws() {
        let mut h = Harness::new();
        h.feed(books("snapshot", S0, 0, &[("4.77", "12827")])).await;
        h.feed(books("update", S0 + 1, S0, &[("4.78", "10")])).await;
        assert_eq!(h.emitted().len(), 2);
        assert!(h.resubs().is_empty());

        h.feed(books("update", S0 + 5, S0 + 4, &[("4.78", "11")])).await;
        assert!(h.emitted().is_empty(), "после разрыва книга не эмитится");
        assert_eq!(h.resubs(), vec![sub_arg("books", INST)]);
        assert!(!h.state().book.ready);

        h.feed(books("update", S0 + 6, S0 + 5, &[("4.78", "12")])).await;
        assert!(h.emitted().is_empty());
        assert!(h.resubs().is_empty(), "одна переподписка на символ, пока ждём снапшот");
    }

    /// Регрессия: после разрыва снапшот переподписки исправляет книгу, и
    /// цепочка дельт продолжается от его seq. Раньше REST-ресинк ставил
    /// `last_seq = timestamp_ms` (~1.8e12 > WS-seq), все дельты отбрасывались,
    /// и фантом 4.77 × 12827 жил до плановой переподписки.
    #[tokio::test]
    async fn regression_resubscribe_snapshot_continues_chain() {
        let mut h = Harness::new();
        h.feed(books("snapshot", S0, 0, &[("4.77", "12827")])).await;
        h.feed(books("update", S0 + 5, S0 + 4, &[("4.78", "11")])).await;
        h.emitted();
        h.resubs();

        let s2 = S0 + 16;
        h.feed(books("snapshot", s2, 0, &[("4.77", "264")])).await;
        let diff = h.emitted();
        assert_eq!(diff.len(), 1);
        assert!(diff[0].asks.iter().any(|l| l.price == dec!(4.77) && l.qty == dec!(264)));
        assert!(h.state().book.ready);
        assert!(h.state().resub.is_none());

        h.feed(books("update", s2 + 1, s2, &[("4.77", "300")])).await;
        let live = h.emitted();
        assert_eq!(live.len(), 1, "дельта после снапшота эмитится");
        assert_eq!(live[0].sequence, Some(s2 + 1));
        assert_eq!(live[0].asks[0].qty, dec!(300));
    }

    /// Потерянная переподписка повторяется с удвоением ожидания, а снапшот
    /// снимает ожидание.
    #[tokio::test]
    async fn stale_resubscribe_is_retried_with_backoff() {
        let mut h = Harness::new();
        let t0 = h.now;
        h.feed(books("snapshot", S0, 0, &[("4.77", "264")])).await;
        h.feed(books("update", S0 + 5, S0 + 4, &[("4.78", "11")])).await;
        assert_eq!(h.resubs().len(), 1);

        retry_stale_resubs(&mut h.states, &h.resub_tx, t0 + Duration::from_secs(5));
        assert!(h.resubs().is_empty());
        retry_stale_resubs(&mut h.states, &h.resub_tx, t0 + RESUB_SNAPSHOT_TIMEOUT);
        assert_eq!(h.resubs().len(), 1);
        retry_stale_resubs(&mut h.states, &h.resub_tx, t0 + Duration::from_secs(29));
        assert!(h.resubs().is_empty(), "второе ожидание вдвое длиннее");
        retry_stale_resubs(&mut h.states, &h.resub_tx, t0 + Duration::from_secs(30));
        assert_eq!(h.resubs().len(), 1);

        h.feed(books("snapshot", S0 + 40, 0, &[("4.77", "264")])).await;
        retry_stale_resubs(&mut h.states, &h.resub_tx, t0 + Duration::from_secs(300));
        assert!(h.resubs().is_empty(), "снапшот снял ожидание");
    }

    /// Дельта символа без снапшота (снапшот потерян) тоже ведёт в
    /// переподписку, а не в REST.
    #[tokio::test]
    async fn delta_before_first_snapshot_requests_resubscribe() {
        let mut h = Harness::new();
        h.feed(books("update", S0 + 1, S0, &[("4.78", "10")])).await;
        assert!(h.emitted().is_empty());
        assert_eq!(h.resubs(), vec![sub_arg("books", INST)]);
    }
}
