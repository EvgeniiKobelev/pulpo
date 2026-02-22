use crate::spot::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_PUBLIC_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const WS_BUSINESS_URL: &str = "wss://ws.okx.com:8443/ws/v5/business";
const EXCHANGE: ExchangeId = ExchangeId::Okx;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to an OKX public WebSocket endpoint, subscribe to the given args,
/// and return a stream of parsed JSON messages that carry a `"data"` field.
///
/// Ping/pong (`"ping"` / `"pong"` text) and subscribe confirmations are
/// automatically filtered out. The connection is re-established with
/// exponential back-off whenever the remote side disconnects.
async fn subscribe_and_stream(
    url: &str,
    args: Vec<serde_json::Value>,
) -> Result<BoxStream<serde_json::Value>> {
    let url_owned = url.to_string();

    let (ws_stream, _) = connect_async(&url_owned)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
    write
        .send(Message::text(sub.to_string()))
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            // Send initial ping.
            let _ = write.send(Message::text("ping".to_string())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
            ping_interval.tick().await; // skip first tick

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
                                // OKX sends literal "pong" text.
                                if text == "pong" {
                                    continue;
                                }
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    // Skip subscribe/unsubscribe confirmations.
                                    if json.get("event").is_some() {
                                        continue;
                                    }
                                    // Only forward messages with a "data" field.
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
                                warn!("OKX WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("OKX WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("OKX WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Reconnect with exponential back-off.
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("OKX WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(&url_owned).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let sub = serde_json::json!({"op": "subscribe", "args": args.clone()});
                        if new_write
                            .send(Message::text(sub.to_string()))
                            .await
                            .is_err()
                        {
                            warn!("OKX WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("OKX WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("OKX WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("OKX WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// Build an OKX subscription arg with instId.
fn sub_arg(channel: &str, inst_id: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        "instId": inst_id
    })
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream order-book snapshots (5 levels) for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let inst_id = unified_to_okx(symbol);
    let arg = sub_arg("books5", &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?;
            let raw: OkxWsBookData = serde_json::from_value(first.clone()).ok()?;
            Some(raw.into_orderbook(EXCHANGE, sym))
        }
    })))
}

impl OkxWsBookData {
    pub fn into_orderbook(self, exchange: ExchangeId, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.ts.parse().unwrap_or(0),
            sequence: self.seq_id,
        }
    }
}

/// Stream real-time trades for a single symbol.
/// OKX sends an array of trades per message; we flatten them.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let inst_id = unified_to_okx(symbol);
    let arg = sub_arg("trades", &inst_id);
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<OkxWsTradeData> =
                    serde_json::from_value(data.clone()).ok()?;
                if !trades.is_empty() {
                    let converted: Vec<Trade> = trades
                        .into_iter()
                        .map(|t| t.into_trade(EXCHANGE))
                        .collect();
                    return Some((futures::stream::iter(converted), stream));
                }
            }
        })
        .flatten(),
    ))
}

/// Stream kline/candlestick updates for a single symbol.
/// OKX candles are served from the `/ws/v5/business` endpoint.
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let inst_id = unified_to_okx(symbol);
    let channel = interval_to_okx_ws(interval);
    let arg = sub_arg(&channel, &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(WS_BUSINESS_URL, vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?.as_array()?;
            let row: Vec<String> = first
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            parse_kline_row(&row, EXCHANGE, &sym, interval)
        }
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream order-book updates for multiple symbols over a single WS connection.
pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("books5", &unified_to_okx(s)))
        .collect();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, args).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let arg = json.get("arg")?;
        let inst_id = arg.get("instId")?.as_str()?;
        let symbol = okx_inst_id_to_unified(inst_id)?;
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: OkxWsBookData = serde_json::from_value(first.clone()).ok()?;
        Some(raw.into_orderbook(EXCHANGE, symbol))
    })))
}

/// Stream real-time trades for multiple symbols over a single WS connection.
pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("trades", &unified_to_okx(s)))
        .collect();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, args).await?;

    Ok(Box::pin(
        futures::stream::unfold(raw_stream, |mut stream| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let trades: Vec<OkxWsTradeData> =
                    serde_json::from_value(data.clone()).ok()?;
                if !trades.is_empty() {
                    let converted: Vec<Trade> = trades
                        .into_iter()
                        .map(|t| t.into_trade(EXCHANGE))
                        .collect();
                    return Some((futures::stream::iter(converted), stream));
                }
            }
        })
        .flatten(),
    ))
}
