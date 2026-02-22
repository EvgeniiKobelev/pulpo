use crate::spot::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, warn};

const WS_URL: &str = "wss://stream.bybit.com/v5/public/spot";

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to the Bybit V5 public spot WebSocket, subscribe to the given
/// topics, and return a [`BoxStream`] that yields parsed JSON values for
/// topic messages only (ping/pong and subscribe confirmations are filtered).
async fn subscribe_and_stream(
    topics: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BybitSpot,
                message: e.to_string(),
            })?;

    let (mut write, mut read) = ws_stream.split();

    // Bybit Spot limits subscribe requests to 10 args each.
    for chunk in topics.chunks(10) {
        let sub = serde_json::json!({"op": "subscribe", "args": chunk});
        write
            .send(Message::text(sub.to_string()))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BybitSpot,
                message: e.to_string(),
            })?;
    }

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        // Handle Bybit text-based ping — respond with pong.
                        if json.get("op").and_then(|v| v.as_str()) == Some("ping") {
                            let pong = serde_json::json!({"op": "pong"});
                            let _ = write.send(Message::text(pong.to_string())).await;
                            continue;
                        }
                        // Skip subscribe confirmation responses.
                        if json.get("op").and_then(|v| v.as_str()) == Some("subscribe") {
                            continue;
                        }
                        // Skip pong responses (our ping reply echoes).
                        if json.get("op").and_then(|v| v.as_str()) == Some("pong") {
                            continue;
                        }
                        // Only forward messages that carry a "topic" field.
                        if json.get("topic").is_some()
                            && tx.send(json).await.is_err()
                        {
                            break;
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    let _ = write.send(Message::Pong(data)).await;
                }
                Ok(Message::Close(_)) => {
                    warn!("Bybit WS connection closed");
                    break;
                }
                Err(e) => {
                    warn!("Bybit WS error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        debug!("Bybit WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream incremental order-book updates for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let topic = format!("orderbook.50.{}", unified_to_bybit(symbol));
    let raw_stream = subscribe_and_stream(vec![topic]).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let data = json.get("data")?;
        let raw: BybitWsOrderBook = serde_json::from_value(data.clone()).ok()?;
        Some(raw.into_orderbook())
    })))
}

/// Stream real-time trades for a single symbol.
///
/// Bybit sends an array of trades per message, so we flatten them into
/// individual `Trade` items.
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
                let trades: Vec<BybitWsTrade> = serde_json::from_value(data.clone()).ok()?;
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

/// Stream kline/candlestick updates for a single symbol.
///
/// Bybit sends an array of kline objects per message; we take the first
/// (latest) entry.
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
            let klines: Vec<BybitWsKlineData> = serde_json::from_value(data.clone()).ok()?;
            let kline = klines.into_iter().next()?;
            Some(kline.into_candle(sym))
        }
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams — single WS connection
// ---------------------------------------------------------------------------

/// Stream order-book updates for multiple symbols over a single WebSocket
/// connection by subscribing to all topics at once.
pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let topics: Vec<String> = symbols
        .iter()
        .map(|s| format!("orderbook.50.{}", unified_to_bybit(s)))
        .collect();
    let raw_stream = subscribe_and_stream(topics).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let data = json.get("data")?;
        let raw: BybitWsOrderBook = serde_json::from_value(data.clone()).ok()?;
        Some(raw.into_orderbook())
    })))
}

/// Stream real-time trades for multiple symbols over a single WebSocket
/// connection by subscribing to all topics at once.
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
                let trades: Vec<BybitWsTrade> = serde_json::from_value(data.clone()).ok()?;
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
