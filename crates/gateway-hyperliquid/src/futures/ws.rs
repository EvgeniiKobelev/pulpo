use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://api.hyperliquid.xyz/ws";

/// Maximum subscriptions per WebSocket connection for batch streams.
///
/// Hyperliquid sends full order-book snapshots for l2Book — keeping subs
/// per connection low avoids overwhelming the connection with data.
const MAX_SUBS_PER_CONNECTION: usize = 30;

/// Delay between individual subscribe messages within a connection.
const SUB_DELAY: Duration = Duration::from_millis(100);

/// Delay between opening successive WS connections for batch streams.
const CONNECTION_DELAY: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to Hyperliquid WebSocket, send subscribe messages **concurrently
/// with reading** (so server responses don't clog the TCP buffer), and return
/// a [`BoxStream`] that yields parsed JSON values.
///
/// - Sends `{"method": "ping"}` every 30 s (Hyperliquid JSON-level ping).
/// - Filters out `subscriptionResponse` and `pong` channel messages.
/// - Auto-reconnects with exponential back-off (1 s → 30 s).
async fn subscribe_stream(
    subscriptions: Vec<serde_json::Value>,
) -> Result<BoxStream<serde_json::Value>> {
    // Just verify we can connect — the background task handles everything.
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::Hyperliquid,
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
    subscriptions: Vec<serde_json::Value>,
    tx: mpsc::Sender<serde_json::Value>,
) {
    let (write, read) = ws_stream.split();
    let mut write = write;
    let mut read = read;
    let mut backoff = Duration::from_secs(5);
    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await;

    // Track how many subscribe messages we still need to send.
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
                                if let Some(ch) = json.get("channel").and_then(|v| v.as_str()) {
                                    if ch == "subscriptionResponse" || ch == "pong" {
                                        continue;
                                    }
                                }
                                if tx.send(json).await.is_err() {
                                    break 'outer;
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                warn!("Hyperliquid WS pong send failed");
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("Hyperliquid WS connection closed");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("Hyperliquid WS error: {}", e);
                            break;
                        }
                        None => {
                            warn!("Hyperliquid WS stream ended unexpectedly");
                            break;
                        }
                        _ => {}
                    }
                }
                // ---- send next subscribe message (interleaved with reads) ----
                _ = sub_delay.tick(), if sub_idx < subscriptions.len() => {
                    let msg = serde_json::json!({
                        "method": "subscribe",
                        "subscription": subscriptions[sub_idx]
                    });
                    if write.send(Message::text(msg.to_string())).await.is_err() {
                        warn!("Hyperliquid WS subscribe send failed at index {}", sub_idx);
                        break;
                    }
                    sub_idx += 1;
                }
                // ---- periodic ping ----
                _ = ping_interval.tick() => {
                    let ping = serde_json::json!({"method": "ping"});
                    if write.send(Message::text(ping.to_string())).await.is_err() {
                        warn!("Hyperliquid WS ping send failed");
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
            warn!("Hyperliquid WS reconnecting in {backoff:?}…");
            tokio::time::sleep(backoff).await;
            match connect_async(WS_URL).await {
                Ok((ws, _)) => {
                    let (new_write, new_read) = ws.split();
                    write = new_write;
                    read = new_read;
                    // Reset subscription index so we re-subscribe to everything.
                    sub_idx = 0;
                    sub_delay = tokio::time::interval(SUB_DELAY);
                    sub_delay.tick().await;
                    backoff = Duration::from_secs(5);
                    ping_interval.reset();
                    info!("Hyperliquid WS reconnected, re-subscribing to {} channels", subscriptions.len());
                    break;
                }
                Err(e) => {
                    warn!("Hyperliquid WS reconnect failed: {}", e);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
    debug!("Hyperliquid WS stream ended");
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream full order-book snapshots for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let sub = serde_json::json!({
        "type": "l2Book",
        "coin": unified_to_hl(symbol)
    });
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let msg: HlWsL2BookMsg = serde_json::from_value(json).ok()?;
        Some(msg.data.into_orderbook())
    })))
}

/// Stream real-time trades for a single symbol.
///
/// Hyperliquid sends an array of trades per message; we flatten them.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let sub = serde_json::json!({
        "type": "trades",
        "coin": unified_to_hl(symbol)
    });
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.flat_map(|json| {
        let trades: Vec<Trade> = serde_json::from_value::<HlWsTradesMsg>(json)
            .map(|msg| msg.data.into_iter().map(|t| t.into_trade()).collect())
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
    let sub = serde_json::json!({
        "type": "candle",
        "coin": unified_to_hl(symbol),
        "interval": interval_to_hl(interval)
    });
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let msg: HlWsCandleMsg = serde_json::from_value(json).ok()?;
        Some(msg.data.into_candle())
    })))
}

/// Stream mark/oracle price updates for a single symbol.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let sub = serde_json::json!({
        "type": "activeAssetCtx",
        "coin": unified_to_hl(symbol)
    });
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let msg: HlWsActiveAssetCtxMsg = serde_json::from_value(json).ok()?;
        Some(msg.data.into_mark_price())
    })))
}

/// Liquidation events are not publicly available on Hyperliquid.
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

/// Stream order-book snapshots for multiple symbols, automatically sharding
/// subscriptions across several WebSocket connections when the count exceeds
/// [`MAX_SUBS_PER_CONNECTION`].
pub async fn stream_orderbooks_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let all_subs: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| serde_json::json!({"type": "l2Book", "coin": unified_to_hl(s)}))
        .collect();

    let chunks: Vec<Vec<serde_json::Value>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "Hyperliquid WS: sharding {} l2Book subs across {} connections (~{} each)",
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
        info!("Hyperliquid WS: opening connection {}/{} ({} subs)", i + 1, n_conns, chunk.len());
        let raw = subscribe_stream(chunk).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(|json| async move {
            let msg: HlWsL2BookMsg = serde_json::from_value(json).ok()?;
            Some(msg.data.into_orderbook())
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
    let all_subs: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| serde_json::json!({"type": "trades", "coin": unified_to_hl(s)}))
        .collect();

    let chunks: Vec<Vec<serde_json::Value>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "Hyperliquid WS: sharding {} trade subs across {} connections (~{} each)",
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
        info!("Hyperliquid WS: opening connection {}/{} ({} subs)", i + 1, n_conns, chunk.len());
        let raw = subscribe_stream(chunk).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.flat_map(|json| {
            let trades: Vec<Trade> = serde_json::from_value::<HlWsTradesMsg>(json)
                .map(|msg| msg.data.into_iter().map(|t| t.into_trade()).collect())
                .unwrap_or_default();
            futures::stream::iter(trades)
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}
