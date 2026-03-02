use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://ws.phemex.com";

/// Maximum subscriptions per WebSocket connection.
///
/// Phemex allows up to 20 subscriptions per connection.
const MAX_SUBS_PER_CONNECTION: usize = 20;

/// Maximum concurrent WS connections per client (Phemex limit: 5).
const MAX_CONNECTIONS: usize = 5;

/// Delay between individual subscribe messages within a connection.
const SUB_DELAY: Duration = Duration::from_millis(100);

/// Delay between opening successive WS connections for batch streams.
const CONNECTION_DELAY: Duration = Duration::from_secs(1);

/// Global atomic counter for WS request IDs.
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// A subscription is a (method, params) pair. E.g. ("orderbook_p.subscribe", ["BTCUSDT"]).
#[derive(Clone, Debug)]
struct Subscription {
    method: String,
    params: Vec<serde_json::Value>,
}

impl Subscription {
    fn to_message(&self) -> String {
        serde_json::json!({
            "id": next_id(),
            "method": &self.method,
            "params": &self.params,
        })
        .to_string()
    }
}

/// Connect to Phemex WebSocket, send subscribe messages concurrently with
/// reading, and return a [`BoxStream`] that yields parsed JSON values.
///
/// - Sends `server.ping` every 9 seconds.
/// - Filters out subscription responses and pong messages.
/// - Auto-reconnects with exponential back-off (1 s → 30 s).
async fn subscribe_stream(
    subscriptions: Vec<Subscription>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(run_ws_loop(ws_stream, subscriptions, tx));

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// The main WS event loop: reads, sends pings, subscribes, and reconnects.
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
    // Phemex requires heartbeat within 30s; CCXT uses 9s interval.
    let mut ping_interval = tokio::time::interval(Duration::from_secs(9));
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
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Filter out subscribe responses and pong
                                if json.get("id").is_some() && json.get("result").is_some() {
                                    // Check for pong or subscribe confirmation
                                    if let Some(result) = json.get("result") {
                                        if result.as_str() == Some("pong") {
                                            continue;
                                        }
                                        // Subscribe confirmations have {"status":"success"}
                                        if result.get("status").is_some() {
                                            continue;
                                        }
                                    }
                                }
                                // Filter out error responses
                                if json.get("error").is_some() && json.get("id").is_some() && !json.get("error").unwrap().is_null() {
                                    warn!("Phemex WS error response: {}", text);
                                    continue;
                                }
                                if tx.send(json).await.is_err() {
                                    break 'outer;
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                warn!("Phemex WS pong send failed");
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("Phemex WS connection closed");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("Phemex WS error: {}", e);
                            break;
                        }
                        None => {
                            warn!("Phemex WS stream ended unexpectedly");
                            break;
                        }
                        _ => {}
                    }
                }
                // ---- send next subscribe message (interleaved with reads) ----
                _ = sub_delay.tick(), if sub_idx < subscriptions.len() => {
                    let msg = subscriptions[sub_idx].to_message();
                    if write.send(Message::text(msg)).await.is_err() {
                        warn!("Phemex WS subscribe send failed at index {}", sub_idx);
                        break;
                    }
                    sub_idx += 1;
                }
                // ---- periodic ping ----
                _ = ping_interval.tick() => {
                    let ping = serde_json::json!({
                        "id": next_id(),
                        "method": "server.ping",
                        "params": []
                    });
                    if write.send(Message::text(ping.to_string())).await.is_err() {
                        warn!("Phemex WS ping send failed");
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
            warn!("Phemex WS reconnecting in {backoff:?}…");
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
                        "Phemex WS reconnected, re-subscribing to {} channels",
                        subscriptions.len()
                    );
                    break;
                }
                Err(e) => {
                    warn!("Phemex WS reconnect failed: {}", e);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
    debug!("Phemex WS stream ended");
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream orderbook snapshots/updates for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let sub = Subscription {
        method: "orderbook_p.subscribe".to_string(),
        params: vec![serde_json::Value::String(unified_to_phemex(symbol))],
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let msg: PhemexWsOrderbookMsg = serde_json::from_value(json).ok()?;
        Some(msg.into_orderbook())
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let sub = Subscription {
        method: "trade_p.subscribe".to_string(),
        params: vec![serde_json::Value::String(unified_to_phemex(symbol))],
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.flat_map(|json| {
        let trades: Vec<Trade> = serde_json::from_value::<PhemexWsTradeMsg>(json)
            .map(|msg| msg.into_trades())
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
    let sub = Subscription {
        method: "kline_p.subscribe".to_string(),
        params: vec![
            serde_json::Value::String(unified_to_phemex(symbol)),
            serde_json::Value::Number(serde_json::Number::from(interval_to_phemex_ws(interval))),
        ],
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.flat_map(|json| {
        let candles: Vec<Candle> = serde_json::from_value::<PhemexWsKlineMsg>(json)
            .map(|msg| msg.into_candles())
            .unwrap_or_default();
        futures::stream::iter(candles)
    })))
}

/// Stream mark price updates for a single symbol via tick_p channel.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let index_symbol = format!(".{}", unified_to_phemex(symbol));
    let sub = Subscription {
        method: "tick_p.subscribe".to_string(),
        params: vec![serde_json::Value::String(index_symbol)],
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        // tick_p messages contain the tick data nested
        let tick: PhemexWsTickMsg = serde_json::from_value(json).ok()?;
        Some(tick.into_mark_price())
    })))
}

/// Liquidation events are not publicly available on Phemex via WebSocket.
/// Returns an empty stream.
pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    _symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    Ok(Box::pin(futures::stream::empty()))
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
            method: "orderbook_p.subscribe".to_string(),
            params: vec![serde_json::Value::String(unified_to_phemex(s))],
        })
        .collect();

    let chunks: Vec<Vec<Subscription>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if symbols.len() < max_total && n_conns > 1 {
        info!(
            "Phemex WS: sharding {} orderbook subs across {} connections (~{} each)",
            all_subs.len(),
            n_conns,
            MAX_SUBS_PER_CONNECTION
        );
    }
    if symbols.len() == max_total {
        info!(
            "Phemex WS: capped at {} symbols ({} connections × {} subs)",
            max_total, MAX_CONNECTIONS, MAX_SUBS_PER_CONNECTION
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for (i, chunk) in chunks.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(CONNECTION_DELAY).await;
        }
        info!(
            "Phemex WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let raw = subscribe_stream(chunk).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(|json| async move {
            let msg: PhemexWsOrderbookMsg = serde_json::from_value(json).ok()?;
            Some(msg.into_orderbook())
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}

/// Stream trades for multiple symbols, automatically sharding subscriptions
/// across several WebSocket connections (up to [`MAX_SUBS_PER_CONNECTION`] each).
pub async fn stream_trades_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let max_total = MAX_SUBS_PER_CONNECTION * MAX_CONNECTIONS;
    let symbols = &symbols[..symbols.len().min(max_total)];

    let all_subs: Vec<Subscription> = symbols
        .iter()
        .map(|s| Subscription {
            method: "trade_p.subscribe".to_string(),
            params: vec![serde_json::Value::String(unified_to_phemex(s))],
        })
        .collect();

    let chunks: Vec<Vec<Subscription>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "Phemex WS: sharding {} trade subs across {} connections (~{} each)",
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
            "Phemex WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let raw = subscribe_stream(chunk).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.flat_map(|json| {
            let trades: Vec<Trade> = serde_json::from_value::<PhemexWsTradeMsg>(json)
                .map(|msg| msg.into_trades())
                .unwrap_or_default();
            futures::stream::iter(trades)
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}
