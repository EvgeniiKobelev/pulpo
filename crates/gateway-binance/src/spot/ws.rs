use crate::spot::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

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

/// Stream order-book depth updates for multiple symbols, automatically
/// sharding subscriptions across several WebSocket connections to stay
/// within Binance limits and avoid connection resets.
pub async fn stream_orderbooks_combined(
    _config: &ExchangeConfig,
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

    let mut select_all = futures::stream::SelectAll::new();
    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(WS_URL, chunk.to_vec()).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(|json| async move {
            let depth_json = json.get("data").cloned().unwrap_or(json);
            let raw: BinanceWsDepthRaw = serde_json::from_value(depth_json).ok()?;
            Some(raw.into_orderbook())
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
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
