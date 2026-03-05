use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://fstream.xt.com/ws/market";

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

/// Connect to XT WebSocket, send subscribe messages, and return a
/// [`BoxStream`] that yields parsed JSON values.
///
/// - Handles XT heartbeat: client sends "ping", server responds "pong".
/// - Filters out subscription confirmations.
/// - Auto-reconnects with exponential back-off (1s -> 30s).
async fn subscribe_stream(
    topics: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(run_ws_loop(ws_stream, topics, tx));

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// The main WS event loop: reads, sends pings, subscribes, and reconnects.
async fn run_ws_loop(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    topics: Vec<String>,
    tx: mpsc::Sender<serde_json::Value>,
) {
    let (write, read) = ws_stream.split();
    let mut write = write;
    let mut read = read;
    let mut backoff = Duration::from_secs(1);
    // Send ping every 25 seconds to keep connection alive (XT requires every 30s)
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
                            // Handle XT pong response (plain text "pong")
                            if text == "pong" {
                                continue;
                            }

                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Filter out subscription confirmations
                                // XT sends back {"id":"...","code":0} for subscribe/unsubscribe
                                if json.get("id").is_some() && json.get("code").is_some() {
                                    continue;
                                }

                                // Forward data messages (have "topic" and "data" fields)
                                if json.get("topic").is_some() && json.get("data").is_some() {
                                    if tx.send(json).await.is_err() {
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                warn!("XT WS pong send failed");
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("XT WS connection closed");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("XT WS error: {}", e);
                            break;
                        }
                        None => {
                            warn!("XT WS stream ended unexpectedly");
                            break;
                        }
                        _ => {}
                    }
                }
                // ---- send next subscribe message (interleaved with reads) ----
                _ = sub_delay.tick(), if sub_idx < topics.len() => {
                    let sub_msg = serde_json::json!({
                        "method": "SUBSCRIBE",
                        "params": [&topics[sub_idx]],
                        "id": format!("sub_{}", sub_idx)
                    }).to_string();
                    if write.send(Message::text(sub_msg)).await.is_err() {
                        warn!("XT WS subscribe send failed at index {}", sub_idx);
                        break;
                    }
                    sub_idx += 1;
                }
                // ---- periodic ping ----
                _ = ping_interval.tick() => {
                    if write.send(Message::text("ping")).await.is_err() {
                        warn!("XT WS ping send failed");
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
            warn!("XT WS reconnecting in {backoff:?}...");
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
                        "XT WS reconnected, re-subscribing to {} channels",
                        topics.len()
                    );
                    break;
                }
                Err(e) => {
                    warn!("XT WS reconnect failed: {}", e);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
    debug!("XT WS stream ended");
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream orderbook snapshots for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let xt_sym = unified_to_xt(symbol);
    let symbol_clone = symbol.clone();
    let topic = format!("depth@{},20,1000ms", xt_sym);
    let raw = subscribe_stream(vec![topic]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            let data = json.get("data")?;
            let depth: XtWsDepthData = serde_json::from_value(data.clone()).ok()?;
            let ts = depth.t.unwrap_or_else(now_ms);
            Some(depth.into_orderbook(&symbol, ts))
        }
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let xt_sym = unified_to_xt(symbol);
    let symbol_clone = symbol.clone();
    let topic = format!("trade@{}", xt_sym);
    let raw = subscribe_stream(vec![topic]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            let data = json.get("data")?;
            let trade_data: XtWsTradeData = serde_json::from_value(data.clone()).ok()?;
            trade_data.into_trade(&symbol)
        }
    })))
}

/// Stream candle updates for a single symbol.
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let xt_sym = unified_to_xt(symbol);
    let symbol_clone = symbol.clone();
    let ws_interval = interval_to_xt_ws(interval);
    let topic = format!("kline@{},{}", xt_sym, ws_interval);
    let raw = subscribe_stream(vec![topic]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            let data = json.get("data")?;
            let kline: XtWsKlineData = serde_json::from_value(data.clone()).ok()?;
            kline.into_candle(&symbol, interval)
        }
    })))
}

/// Stream mark price updates for a single symbol.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let xt_sym = unified_to_xt(symbol);
    let symbol_clone = symbol.clone();
    let topic = format!("mark_price@{}", xt_sym);
    let raw = subscribe_stream(vec![topic]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            let data = json.get("data")?;
            let mp: XtWsMarkPriceData = serde_json::from_value(data.clone()).ok()?;
            Some(mp.into_mark_price(&symbol, Decimal::ZERO))
        }
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

    let all_subs: Vec<(String, Symbol)> = symbols
        .iter()
        .map(|s| {
            let xt_sym = unified_to_xt(s);
            (format!("depth@{},20,1000ms", xt_sym), s.clone())
        })
        .collect();

    let chunks: Vec<Vec<(String, Symbol)>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "XT WS: sharding {} orderbook subs across {} connections (~{} each)",
            symbols.len(),
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
            "XT WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let topics: Vec<String> = chunk.iter().map(|(t, _)| t.clone()).collect();
        let sym_map: std::collections::HashMap<String, Symbol> = chunk
            .into_iter()
            .map(|(topic, sym)| {
                // Extract xt_sym from topic "depth@btc_usdt,20,1000ms"
                let xt_sym = topic
                    .strip_prefix("depth@")
                    .and_then(|s| s.split(',').next())
                    .unwrap_or("")
                    .to_string();
                (xt_sym, sym)
            })
            .collect();

        let raw = subscribe_stream(topics).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(move |json| {
            let sym_map = sym_map.clone();
            async move {
                let event = json.get("event")?.as_str()?;
                // event looks like "depth@btc_usdt,20"
                let xt_sym = event
                    .strip_prefix("depth@")
                    .and_then(|s| s.split(',').next())?;
                let symbol = sym_map.get(xt_sym)?;
                let data = json.get("data")?;
                let depth: XtWsDepthData = serde_json::from_value(data.clone()).ok()?;
                let ts = depth.t.unwrap_or_else(now_ms);
                Some(depth.into_orderbook(symbol, ts))
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

    let all_subs: Vec<(String, Symbol)> = symbols
        .iter()
        .map(|s| {
            let xt_sym = unified_to_xt(s);
            (format!("trade@{}", xt_sym), s.clone())
        })
        .collect();

    let chunks: Vec<Vec<(String, Symbol)>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "XT WS: sharding {} trade subs across {} connections (~{} each)",
            symbols.len(),
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
            "XT WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let topics: Vec<String> = chunk.iter().map(|(t, _)| t.clone()).collect();
        let sym_map: std::collections::HashMap<String, Symbol> = chunk
            .into_iter()
            .map(|(topic, sym)| {
                let xt_sym = topic
                    .strip_prefix("trade@")
                    .unwrap_or("")
                    .to_string();
                (xt_sym, sym)
            })
            .collect();

        let raw = subscribe_stream(topics).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.filter_map(move |json| {
            let sym_map = sym_map.clone();
            async move {
                let event = json.get("event")?.as_str()?;
                // event looks like "trade@btc_usdt"
                let xt_sym = event.strip_prefix("trade@")?;
                let symbol = sym_map.get(xt_sym)?;
                let data = json.get("data")?;
                let trade_data: XtWsTradeData = serde_json::from_value(data.clone()).ok()?;
                trade_data.into_trade(symbol)
            }
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}

use rust_decimal::Decimal;
