use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, warn};

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
async fn subscribe_and_stream(
    url: &str,
    streams: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(url)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            })?;

    let (mut write, mut read) = ws_stream.split();

    // Send SUBSCRIBE message when using the single-stream endpoint.
    if !streams.is_empty() {
        let sub = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": streams,
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
        // Keep `write` alive so the connection is not half-closed.
        let _write = write;
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        // Skip subscription confirmation responses like {"result":null,"id":1}
                        if json.get("result").is_some() && json.get("id").is_some() {
                            continue;
                        }
                        if tx.send(json).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("Binance Futures WS connection closed");
                    break;
                }
                Err(e) => {
                    warn!("Binance Futures WS error: {}", e);
                    break;
                }
                _ => {}
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
pub async fn stream_orderbooks_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@depth@100ms", unified_to_binance(s).to_lowercase()))
        .collect();
    let streams_path = streams.join("/");
    let url = format!("{}?streams={}", COMBINED_WS_URL, streams_path);

    // Combined-stream URLs carry the subscription in the query string, so we
    // pass an empty vec (no explicit SUBSCRIBE message needed).
    let raw = subscribe_and_stream(&url, vec![]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        // Combined stream wraps data: {"stream":"...","data":{...}}
        let data = json.get("data")?.clone();
        let raw: BinanceFuturesWsDepthRaw = serde_json::from_value(data).ok()?;
        Some(raw.into_orderbook())
    })))
}

/// Stream real-time trades for multiple symbols over a single combined
/// WebSocket connection.
pub async fn stream_trades_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@trade", unified_to_binance(s).to_lowercase()))
        .collect();
    let streams_path = streams.join("/");
    let url = format!("{}?streams={}", COMBINED_WS_URL, streams_path);

    let raw = subscribe_and_stream(&url, vec![]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let data = json.get("data")?.clone();
        let raw: BinanceFuturesWsTradeRaw = serde_json::from_value(data).ok()?;
        Some(raw.into_trade())
    })))
}
