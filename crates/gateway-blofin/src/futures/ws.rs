use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://openapi.blofin.com/ws/public";

/// Maximum subscriptions per WebSocket connection.
const MAX_SUBS_PER_CONNECTION: usize = 50;

/// Maximum concurrent WS connections per client.
const MAX_CONNECTIONS: usize = 5;

/// Delay between individual subscribe messages within a connection.
const SUB_DELAY: Duration = Duration::from_millis(100);

/// Delay between opening successive WS connections for batch streams.
const CONNECTION_DELAY: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// A subscription is a channel + instId pair.
#[derive(Clone, Debug)]
struct Subscription {
    channel: String,
    inst_id: String,
}

impl Subscription {
    fn to_message(&self) -> String {
        serde_json::json!({
            "op": "subscribe",
            "args": [{
                "channel": &self.channel,
                "instId": &self.inst_id,
            }]
        })
        .to_string()
    }
}

/// Connect to BloFin WebSocket, send subscribe messages concurrently with
/// reading, and return a [`BoxStream`] that yields parsed JSON values.
///
/// - Sends `"pong"` in response to `"ping"` messages (BloFin heartbeat).
/// - Filters out subscription confirmations.
/// - Auto-reconnects with exponential back-off (1 s -> 30 s).
async fn subscribe_stream(
    subscriptions: Vec<Subscription>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(run_ws_loop(ws_stream, subscriptions, tx));

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// The main WS event loop: reads, sends pongs, subscribes, and reconnects.
async fn run_ws_loop(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    subscriptions: Vec<Subscription>,
    tx: mpsc::Sender<serde_json::Value>,
) {
    let (write, read) = ws_stream.split();
    let mut write = write;
    let mut read = read;
    let mut backoff = Duration::from_secs(1);
    // BloFin requires pong within 30s; we send proactive ping every 25s.
    let mut ping_interval = tokio::time::interval(Duration::from_secs(25));
    ping_interval.tick().await;

    let mut sub_idx: usize = 0;
    let mut sub_delay = tokio::time::interval(SUB_DELAY);
    sub_delay.tick().await;

    'outer: loop {
        loop {
            tokio::select! {
                // ---- read data ----
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            // BloFin sends plain "ping" string as heartbeat
                            if AsRef::<str>::as_ref(&text) == "ping" {
                                if write.send(Message::text("pong")).await.is_err() {
                                    warn!("BloFin WS pong send failed");
                                    break;
                                }
                                continue;
                            }
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Filter out subscribe/unsubscribe confirmations
                                if let Some(event) = json.get("event").and_then(|e| e.as_str()) {
                                    match event {
                                        "subscribe" | "unsubscribe" => continue,
                                        "error" => {
                                            warn!("BloFin WS error: {}", text);
                                            continue;
                                        }
                                        _ => {}
                                    }
                                }
                                // Only forward messages with actual data
                                if json.get("data").is_some() {
                                    if tx.send(json).await.is_err() {
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                warn!("BloFin WS pong send failed");
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("BloFin WS connection closed");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("BloFin WS error: {}", e);
                            break;
                        }
                        None => {
                            warn!("BloFin WS stream ended unexpectedly");
                            break;
                        }
                        _ => {}
                    }
                }
                // ---- send next subscribe message (interleaved with reads) ----
                _ = sub_delay.tick(), if sub_idx < subscriptions.len() => {
                    let msg = subscriptions[sub_idx].to_message();
                    if write.send(Message::text(msg)).await.is_err() {
                        warn!("BloFin WS subscribe send failed at index {}", sub_idx);
                        break;
                    }
                    sub_idx += 1;
                }
                // ---- periodic ping ----
                _ = ping_interval.tick() => {
                    if write.send(Message::text("ping")).await.is_err() {
                        warn!("BloFin WS ping send failed");
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
            warn!("BloFin WS reconnecting in {backoff:?}...");
            tokio::time::sleep(backoff).await;
            match connect_async(WS_URL).await {
                Ok((ws, _)) => {
                    let (new_write, new_read) = ws.split();
                    write = new_write;
                    read = new_read;
                    sub_idx = 0;
                    sub_delay = tokio::time::interval(SUB_DELAY);
                    sub_delay.tick().await;
                    backoff = Duration::from_secs(1);
                    ping_interval.reset();
                    info!(
                        "BloFin WS reconnected, re-subscribing to {} channels",
                        subscriptions.len()
                    );
                    break;
                }
                Err(e) => {
                    warn!("BloFin WS reconnect failed: {}", e);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
    debug!("BloFin WS stream ended");
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream orderbook snapshots/updates for a single symbol.
///
/// Uses `books5` channel (top 5 levels, full snapshot every 100ms).
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let inst_id = unified_to_blofin(symbol);
    let sub = Subscription {
        channel: "books5".to_string(),
        inst_id: inst_id.clone(),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let inst_id = inst_id.clone();
        async move {
            let data = json.get("data")?;
            // BloFin books5 returns data as an object, not an array
            let ob: BlofinWsOrderbookData = serde_json::from_value(data.clone()).ok()?;
            Some(ob.into_orderbook(&inst_id))
        }
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let sub = Subscription {
        channel: "trades".to_string(),
        inst_id: unified_to_blofin(symbol),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.flat_map(|json| {
        let trades: Vec<Trade> = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        serde_json::from_value::<BlofinWsTradeData>(v.clone())
                            .ok()
                            .and_then(|t| t.into_trade())
                    })
                    .collect()
            })
            .unwrap_or_default();
        futures::stream::iter(trades)
    })))
}

/// Stream candle updates for a single symbol.
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let inst_id = unified_to_blofin(symbol);
    let bar = interval_to_blofin_ws(interval);
    let sub = Subscription {
        channel: format!("candle{}", bar),
        inst_id: inst_id.clone(),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let inst_id = inst_id.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?;
            let candle_data: BlofinWsCandleData = serde_json::from_value(first.clone()).ok()?;
            candle_data.into_candle(&inst_id, interval)
        }
    })))
}

/// Stream mark price updates for a single symbol via tickers channel.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let sub = Subscription {
        channel: "tickers".to_string(),
        inst_id: unified_to_blofin(symbol),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let ticker: BlofinWsTickerData = serde_json::from_value(first.clone()).ok()?;
        Some(ticker.into_mark_price())
    })))
}

// ---------------------------------------------------------------------------
// Combined (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream orderbook updates for multiple symbols, automatically sharding
/// subscriptions across several WebSocket connections when the count exceeds
/// [`MAX_SUBS_PER_CONNECTION`].
pub async fn stream_orderbooks_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let max_total = MAX_SUBS_PER_CONNECTION * MAX_CONNECTIONS;
    let symbols = &symbols[..symbols.len().min(max_total)];

    let all_subs: Vec<Subscription> = symbols
        .iter()
        .map(|s| Subscription {
            channel: "books5".to_string(),
            inst_id: unified_to_blofin(s),
        })
        .collect();

    let chunks: Vec<Vec<Subscription>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if symbols.len() < max_total && n_conns > 1 {
        info!(
            "BloFin WS: sharding {} orderbook subs across {} connections (~{} each)",
            all_subs.len(),
            n_conns,
            MAX_SUBS_PER_CONNECTION
        );
    }
    if symbols.len() == max_total {
        info!(
            "BloFin WS: capped at {} symbols ({} connections x {} subs)",
            max_total, MAX_CONNECTIONS, MAX_SUBS_PER_CONNECTION
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for (i, chunk) in chunks.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(CONNECTION_DELAY).await;
        }
        info!(
            "BloFin WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let inst_ids: Vec<String> = chunk.iter().map(|s| s.inst_id.clone()).collect();
        let raw = subscribe_stream(chunk).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(move |json| {
            let inst_ids = inst_ids.clone();
            async move {
                let arg = json.get("arg")?;
                let inst_id = arg.get("instId")?.as_str()?;
                // Verify the inst_id is one of ours
                if !inst_ids.iter().any(|id| id == inst_id) {
                    return None;
                }
                let data = json.get("data")?;
                // BloFin books5 returns data as an object, not an array
                let ob: BlofinWsOrderbookData = serde_json::from_value(data.clone()).ok()?;
                Some(ob.into_orderbook(inst_id))
            }
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}

/// Stream trades for multiple symbols, automatically sharding subscriptions
/// across several WebSocket connections.
pub async fn stream_trades_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let max_total = MAX_SUBS_PER_CONNECTION * MAX_CONNECTIONS;
    let symbols = &symbols[..symbols.len().min(max_total)];

    let all_subs: Vec<Subscription> = symbols
        .iter()
        .map(|s| Subscription {
            channel: "trades".to_string(),
            inst_id: unified_to_blofin(s),
        })
        .collect();

    let chunks: Vec<Vec<Subscription>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "BloFin WS: sharding {} trade subs across {} connections (~{} each)",
            all_subs.len(),
            n_conns,
            MAX_SUBS_PER_CONNECTION
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for (i, chunk) in chunks.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(CONNECTION_DELAY).await;
        }
        info!(
            "BloFin WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let raw = subscribe_stream(chunk).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.flat_map(|json| {
            let trades: Vec<Trade> = json
                .get("data")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            serde_json::from_value::<BlofinWsTradeData>(v.clone())
                                .ok()
                                .and_then(|t| t.into_trade())
                        })
                        .collect()
                })
                .unwrap_or_default();
            futures::stream::iter(trades)
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}
