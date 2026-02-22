use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://ws.bitget.com/v2/ws/public";

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to the Bitget V2 public futures WebSocket, subscribe to the given
/// args, and return a [`BoxStream`] that yields parsed JSON `"data"` arrays
/// for topic messages only (ping/pong and subscribe confirmations are filtered).
///
/// The connection is automatically re-established with exponential back-off
/// whenever the remote side disconnects.
async fn subscribe_and_stream(
    args: Vec<serde_json::Value>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::BitgetFutures,
                message: e.to_string(),
            })?;

    let (mut write, read) = ws_stream.split();

    // Send SUBSCRIBE message.
    let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
    write
        .send(Message::text(sub.to_string()))
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: ExchangeId::BitgetFutures,
            message: e.to_string(),
        })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            // Send initial ping for this connection.
            let _ = write.send(Message::text("ping".to_string())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
            ping_interval.tick().await; // skip first tick

            // ---- message read loop with periodic ping ----
            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text("ping".to_string())).await.is_err() {
                            break; // reconnect
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                // Bitget sends literal "pong" text in response to "ping"
                                if text == "pong" {
                                    continue;
                                }
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    // Skip subscribe confirmation responses.
                                    if json.get("event").and_then(|v| v.as_str()) == Some("subscribe") {
                                        continue;
                                    }
                                    // Only forward messages that carry a "data" field.
                                    if json.get("data").is_some()
                                        && tx.send(json).await.is_err()
                                    {
                                        break 'outer;
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Bitget futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Bitget futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Bitget futures WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            // ---- reconnect with exponential back-off ----
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Bitget futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
                        if new_write
                            .send(Message::text(sub.to_string()))
                            .await
                            .is_err()
                        {
                            warn!("Bitget futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Bitget futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Bitget futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Bitget futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// Build a Bitget WS subscription arg object for USDT-FUTURES.
fn sub_arg(channel: &str, inst_id: &str) -> serde_json::Value {
    serde_json::json!({
        "instType": "USDT-FUTURES",
        "channel": channel,
        "instId": inst_id
    })
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream order-book snapshots for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("books5", &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?;
            let raw: BitgetMixWsOrderBook = serde_json::from_value(first.clone()).ok()?;
            Some(raw.into_orderbook(sym))
        }
    })))
}

/// Stream real-time trades for a single symbol.
///
/// Bitget sends an array of trades per message, so we flatten them into
/// individual `Trade` items.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("trade", &inst_id);
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<BitgetMixWsTradeRaw> =
                    serde_json::from_value(data.clone()).ok()?;
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
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let inst_id = unified_to_bitget(symbol);
    let channel = interval_to_bitget_ws(interval);
    let arg = sub_arg(channel, &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?.as_array()?;
            let row: Vec<String> = first
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            parse_ws_kline(&row, sym)
        }
    })))
}

// ---------------------------------------------------------------------------
// Futures-specific streams
// ---------------------------------------------------------------------------

/// Stream mark price updates for a single symbol.
///
/// Subscribes to the `ticker` channel and extracts mark_price/index_price.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("ticker", &inst_id);
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?;
            let raw: BitgetMixWsTickerRaw = serde_json::from_value(first.clone()).ok()?;
            Some(raw.into_mark_price())
        }
    })))
}

/// Stream liquidation events for a single symbol.
///
/// Subscribes to the `liquidation` channel.
pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let inst_id = unified_to_bitget(symbol);
    let arg = sub_arg("liquidation", &inst_id);
    let raw_stream = subscribe_and_stream(vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?;
            let raw: BitgetMixWsLiquidationRaw = serde_json::from_value(first.clone()).ok()?;
            Some(raw.into_liquidation())
        }
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams -- single WS connection
// ---------------------------------------------------------------------------

/// Stream order-book updates for multiple symbols over a single WebSocket
/// connection by subscribing to all topics at once.
pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("books5", &unified_to_bitget(s)))
        .collect();
    let raw_stream = subscribe_and_stream(args).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let arg = json.get("arg")?;
        let inst_id = arg.get("instId")?.as_str()?;
        let symbol = bitget_symbol_to_unified(inst_id);
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: BitgetMixWsOrderBook = serde_json::from_value(first.clone()).ok()?;
        Some(raw.into_orderbook(symbol))
    })))
}

/// Stream real-time trades for multiple symbols over a single WebSocket
/// connection by subscribing to all topics at once.
pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("trade", &unified_to_bitget(s)))
        .collect();
    let raw_stream = subscribe_and_stream(args).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<BitgetMixWsTradeRaw> =
                    serde_json::from_value(data.clone()).ok()?;
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
