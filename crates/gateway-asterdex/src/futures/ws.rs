use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://fstream.asterdex.com/ws";
const COMBINED_WS_URL: &str = "wss://fstream.asterdex.com/stream";

/// Maximum number of streams per single WebSocket connection.
const MAX_STREAMS_PER_CONNECTION: usize = 100;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to an Asterdex Futures WebSocket endpoint, optionally send a
/// SUBSCRIBE message, and return a [`BoxStream`] that yields parsed JSON values.
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
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            })?;

    let (mut write, read) = ws_stream.split();

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
                exchange: ExchangeId::AsterdexFutures,
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
                                    warn!("Asterdex Futures WS pong send failed");
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Asterdex Futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Asterdex Futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Asterdex Futures WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = ping_interval.tick() => {
                        if write.send(Message::Ping(vec![].into())).await.is_err() {
                            warn!("Asterdex Futures WS ping send failed");
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
                warn!("Asterdex Futures WS reconnecting in {backoff:?}…");
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
                                warn!("Asterdex Futures WS subscribe failed after reconnect");
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                continue;
                            }
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        ping_interval.reset();
                        info!("Asterdex Futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Asterdex Futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Asterdex Futures WS stream ended");
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
    let stream_name = format!("{}@depth@100ms", unified_to_asterdex(symbol).to_lowercase());
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: AsterdexWsDepthRaw = serde_json::from_value(json).ok()?;
        Some(raw.into_orderbook())
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let stream_name = format!("{}@aggTrade", unified_to_asterdex(symbol).to_lowercase());
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: AsterdexWsTradeRaw = serde_json::from_value(json).ok()?;
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
        unified_to_asterdex(symbol).to_lowercase(),
        interval_to_asterdex(interval)
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: AsterdexWsKlineMsg = serde_json::from_value(json).ok()?;
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
        unified_to_asterdex(symbol).to_lowercase()
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: AsterdexWsMarkPriceRaw = serde_json::from_value(json).ok()?;
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
        unified_to_asterdex(symbol).to_lowercase()
    );
    let raw = subscribe_and_stream(WS_URL, vec![stream_name]).await?;

    Ok(Box::pin(raw.filter_map(|json| async move {
        let raw: AsterdexWsForceOrderMsg = serde_json::from_value(json).ok()?;
        Some(raw.into_liquidation())
    })))
}

// ---------------------------------------------------------------------------
// Combined (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream order-book depth updates for multiple symbols, automatically
/// sharding subscriptions across several WebSocket connections.
pub async fn stream_orderbooks_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let all_streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@depth@100ms", unified_to_asterdex(s).to_lowercase()))
        .collect();

    let num_connections = all_streams.chunks(MAX_STREAMS_PER_CONNECTION).len();
    if num_connections > 1 {
        info!(
            "Asterdex Futures WS: sharding {} depth streams across {} connections",
            all_streams.len(),
            num_connections
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(COMBINED_WS_URL, chunk.to_vec()).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(|json| async move {
            let data = json.get("data")?.clone();
            let raw: AsterdexWsDepthRaw = serde_json::from_value(data).ok()?;
            Some(raw.into_orderbook())
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}

/// Stream real-time trades for multiple symbols, automatically
/// sharding subscriptions across several WebSocket connections.
pub async fn stream_trades_combined(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let all_streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@aggTrade", unified_to_asterdex(s).to_lowercase()))
        .collect();

    let num_connections = all_streams.chunks(MAX_STREAMS_PER_CONNECTION).len();
    if num_connections > 1 {
        info!(
            "Asterdex Futures WS: sharding {} trade streams across {} connections",
            all_streams.len(),
            num_connections
        );
    }

    let mut select_all = futures::stream::SelectAll::new();
    for chunk in all_streams.chunks(MAX_STREAMS_PER_CONNECTION) {
        let raw = subscribe_and_stream(COMBINED_WS_URL, chunk.to_vec()).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.filter_map(|json| async move {
            let data = json.get("data")?.clone();
            let raw: AsterdexWsTradeRaw = serde_json::from_value(data).ok()?;
            Some(raw.into_trade())
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}
