use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://fstream.binance.com/ws";
const COMBINED_WS_URL: &str = "wss://fstream.binance.com/stream";

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to a Binance Futures WebSocket endpoint, optionally send a SUBSCRIBE
/// message, and return a [`BoxStream`] that yields parsed JSON values.
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

    let (ws_stream, _) =
        connect_async(&url)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BinanceFutures,
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
                exchange: ExchangeId::BinanceFutures,
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
                                    warn!("Binance Futures WS pong send failed");
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Binance Futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Binance Futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Binance Futures WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = ping_interval.tick() => {
                        if write.send(Message::Ping(vec![].into())).await.is_err() {
                            warn!("Binance Futures WS ping send failed");
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
                warn!("Binance Futures WS reconnecting in {backoff:?}…");
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
                                warn!("Binance Futures WS subscribe failed after reconnect");
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                continue;
                            }
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        ping_interval.reset();
                        info!("Binance Futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Binance Futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Binance Futures WS stream ended");
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
        let raw: BinanceFuturesWsDepthRaw = serde_json::from_value(json).ok()?;
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
        let raw: BinanceFuturesWsTradeRaw = serde_json::from_value(json).ok()?;
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
        let raw: BinanceFuturesWsKlineMsg = serde_json::from_value(json).ok()?;
        Some(raw.into_candle())
    })))
}

/// Stream mark price updates for a single symbol (1s interval).
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let stream_name = format!(
        "{}@markPrice@1s",
        unified_to_binance(symbol).to_lowercase()
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceWsMarkPriceRaw = serde_json::from_value(json).ok()?;
        Some(raw.into_mark_price())
    })))
}

/// Stream liquidation (force order) events for a single symbol.
pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let stream_name = format!(
        "{}@forceOrder",
        unified_to_binance(symbol).to_lowercase()
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: BinanceWsForceOrderMsg = serde_json::from_value(json).ok()?;
        Some(raw.into_liquidation())
    })))
}

// ---------------------------------------------------------------------------
// Combined (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream order-book depth updates for multiple symbols over a single
/// combined WebSocket connection.
///
/// Uses the `/stream` endpoint with SUBSCRIBE method instead of URL query
/// params to avoid URL-length issues with many symbols.
pub async fn stream_orderbooks_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@depth@100ms", unified_to_binance(s).to_lowercase()))
        .collect();

    let raw = subscribe_and_stream(COMBINED_WS_URL, streams).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        // Combined stream wraps data: {"stream":"...","data":{...}}
        let data = json.get("data")?.clone();
        let raw: BinanceFuturesWsDepthRaw = serde_json::from_value(data).ok()?;
        Some(raw.into_orderbook())
    })))
}

/// Stream real-time trades for multiple symbols over a single combined
/// WebSocket connection.
///
/// Uses the `/stream` endpoint with SUBSCRIBE method instead of URL query
/// params to avoid URL-length issues with many symbols.
pub async fn stream_trades_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@trade", unified_to_binance(s).to_lowercase()))
        .collect();

    let raw = subscribe_and_stream(COMBINED_WS_URL, streams).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let data = json.get("data")?.clone();
        let raw: BinanceFuturesWsTradeRaw = serde_json::from_value(data).ok()?;
        Some(raw.into_trade())
    })))
}
