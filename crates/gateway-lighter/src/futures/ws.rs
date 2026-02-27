use crate::futures::mapper::*;
use futures::{stream::SelectAll, SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://mainnet.zklighter.elliot.ai/stream";

/// Maximum number of channel subscriptions per WebSocket connection.
/// Lighter allows 100 per connection; we use 50 to stay safely within limits.
const CHUNK_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to the Lighter public WebSocket, subscribe to the given channels,
/// and return a [`BoxStream`] of raw JSON values.
///
/// Lighter subscribes one channel per message:
/// ```json
/// {"type": "subscribe", "channel": "trade/0"}
/// ```
///
/// The connection is automatically re-established with exponential back-off
/// whenever the remote side disconnects.
async fn subscribe_and_stream(
    channels: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
                message: e.to_string(),
            })?;

    let (mut write, read) = ws_stream.split();

    // Send subscribe for each channel.
    for channel in &channels {
        let sub = serde_json::json!({"type": "subscribe", "channel": channel});
        write
            .send(Message::text(sub.to_string()))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
                message: e.to_string(),
            })?;
    }

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            // ---- message read loop ----
            loop {
                match read.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(json) =
                            serde_json::from_str::<serde_json::Value>(&text)
                        {
                            // Skip subscription confirmations.
                            if json.get("type").and_then(|v| v.as_str())
                                == Some("subscribed")
                            {
                                continue;
                            }
                            // Skip error messages (log them).
                            if json.get("type").and_then(|v| v.as_str())
                                == Some("error")
                            {
                                warn!(
                                    "Lighter WS error: {}",
                                    json.get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                );
                                continue;
                            }
                            // Forward update messages.
                            if tx.send(json).await.is_err() {
                                break 'outer;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        warn!("Lighter WS connection closed");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("Lighter WS error: {}", e);
                        break;
                    }
                    None => {
                        warn!("Lighter WS stream ended unexpectedly");
                        break;
                    }
                    _ => {}
                }
            }

            // ---- reconnect with exponential back-off ----
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Lighter WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        // Resubscribe to all channels.
                        let mut ok = true;
                        for channel in &channels {
                            let sub = serde_json::json!({
                                "type": "subscribe",
                                "channel": channel
                            });
                            if new_write
                                .send(Message::text(sub.to_string()))
                                .await
                                .is_err()
                            {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("Lighter WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Lighter WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Lighter WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Lighter WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream order-book updates for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    market_id: u16,
    symbol: Symbol,
) -> Result<BoxStream<OrderBook>> {
    let channel = format!("order_book/{}", market_id);
    let raw_stream = subscribe_and_stream(vec![channel]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let symbol = symbol.clone();
        async move {
            let ob_data = json.get("order_book")?;
            let raw: LighterWsOrderBookUpdate =
                serde_json::from_value(ob_data.clone()).ok()?;
            Some(raw.into_orderbook(symbol))
        }
    })))
}

/// Stream real-time trades for a single symbol.
///
/// Lighter sends an array of trades per message, so we flatten them.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    market_id: u16,
    symbol: Symbol,
) -> Result<BoxStream<Trade>> {
    let channel = format!("trade/{}", market_id);
    let raw_stream = subscribe_and_stream(vec![channel]).await?;

    Ok(Box::pin(
        futures::stream::unfold(
            (raw_stream, symbol),
            |(mut stream, symbol)| async move {
                loop {
                    let json = stream.next().await?;
                    let Some(trades_data) = json.get("trades") else {
                        continue;
                    };
                    let Ok(trades) =
                        serde_json::from_value::<Vec<LighterWsTrade>>(trades_data.clone())
                    else {
                        continue;
                    };
                    if !trades.is_empty() {
                        let converted: Vec<Trade> = trades
                            .into_iter()
                            .map(|t| t.into_trade(symbol.clone()))
                            .collect();
                        return Some((
                            futures::stream::iter(converted),
                            (stream, symbol),
                        ));
                    }
                }
            },
        )
        .flatten(),
    ))
}

/// Stream mark price updates via the market_stats channel.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    market_id: u16,
    symbol: Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let channel = format!("market_stats/{}", market_id);
    let raw_stream = subscribe_and_stream(vec![channel]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let symbol = symbol.clone();
        async move {
            let stats_data = json.get("market_stats")?;
            let raw: LighterWsMarketStats =
                serde_json::from_value(stats_data.clone()).ok()?;
            Some(raw.into_mark_price(symbol))
        }
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams – chunked across connections
// ---------------------------------------------------------------------------

/// Stream order-book updates for multiple symbols.
///
/// Subscriptions are chunked at [CHUNK_SIZE] channels per WebSocket
/// connection, and the resulting streams are merged via [`SelectAll`].
pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    market_ids: &[(u16, Symbol)],
) -> Result<BoxStream<OrderBook>> {
    let mut all = SelectAll::new();

    for chunk in market_ids.chunks(CHUNK_SIZE) {
        let channels: Vec<String> = chunk
            .iter()
            .map(|(mid, _)| format!("order_book/{}", mid))
            .collect();

        // Build a lookup map for this chunk: market_id → Symbol
        let id_to_symbol: std::collections::HashMap<u16, Symbol> = chunk
            .iter()
            .map(|(mid, sym)| (*mid, sym.clone()))
            .collect();

        let raw_stream = subscribe_and_stream(channels).await?;

        let chunk_stream: BoxStream<OrderBook> =
            Box::pin(raw_stream.filter_map(move |json| {
                let id_to_symbol = id_to_symbol.clone();
                async move {
                    let channel = json.get("channel")?.as_str()?;
                    let market_id = parse_market_id_from_channel(channel)?;
                    let symbol = id_to_symbol.get(&market_id)?.clone();
                    let ob_data = json.get("order_book")?;
                    let raw: LighterWsOrderBookUpdate =
                        serde_json::from_value(ob_data.clone()).ok()?;
                    Some(raw.into_orderbook(symbol))
                }
            }));

        all.push(chunk_stream);
    }

    Ok(Box::pin(all))
}

/// Stream real-time trades for multiple symbols.
///
/// Subscriptions are chunked at [CHUNK_SIZE] channels per WebSocket
/// connection, and the resulting streams are merged via [`SelectAll`].
pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    market_ids: &[(u16, Symbol)],
) -> Result<BoxStream<Trade>> {
    let mut all = SelectAll::new();

    for chunk in market_ids.chunks(CHUNK_SIZE) {
        let channels: Vec<String> = chunk
            .iter()
            .map(|(mid, _)| format!("trade/{}", mid))
            .collect();

        let id_to_symbol: std::collections::HashMap<u16, Symbol> = chunk
            .iter()
            .map(|(mid, sym)| (*mid, sym.clone()))
            .collect();

        let raw_stream = subscribe_and_stream(channels).await?;

        let chunk_stream: BoxStream<Trade> = Box::pin(
            futures::stream::unfold(
                (raw_stream, id_to_symbol),
                |(mut stream, id_to_symbol)| async move {
                    loop {
                        let json = stream.next().await?;
                        let Some(channel) =
                            json.get("channel").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        let Some(market_id) = parse_market_id_from_channel(channel)
                        else {
                            continue;
                        };
                        let Some(symbol) = id_to_symbol.get(&market_id).cloned()
                        else {
                            continue;
                        };
                        let Some(trades_data) = json.get("trades") else {
                            continue;
                        };
                        let Ok(trades) = serde_json::from_value::<Vec<LighterWsTrade>>(
                            trades_data.clone(),
                        ) else {
                            continue;
                        };
                        if !trades.is_empty() {
                            let converted: Vec<Trade> = trades
                                .into_iter()
                                .map(|t| t.into_trade(symbol.clone()))
                                .collect();
                            return Some((
                                futures::stream::iter(converted),
                                (stream, id_to_symbol),
                            ));
                        }
                    }
                },
            )
            .flatten(),
        );

        all.push(chunk_stream);
    }

    Ok(Box::pin(all))
}
