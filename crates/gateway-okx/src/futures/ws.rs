use crate::futures::mapper::*;
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
// Core helper — same pattern as spot ws
// ---------------------------------------------------------------------------

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
            let _ = write.send(Message::text("ping".to_string())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
            ping_interval.tick().await;

            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text("ping".to_string())).await.is_err() {
                            break;
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if text == "pong" {
                                    continue;
                                }
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("event").is_some() {
                                        continue;
                                    }
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
                                warn!("OKX futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("OKX futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("OKX futures WS stream ended unexpectedly");
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
                warn!("OKX futures WS reconnecting in {backoff:?}…");
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
                            warn!("OKX futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("OKX futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("OKX futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("OKX futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

fn sub_arg(channel: &str, inst_id: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        "instId": inst_id
    })
}

fn sub_arg_inst_type(channel: &str, inst_type: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        "instType": inst_type
    })
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let inst_id = unified_to_okx_swap(symbol);
    let arg = sub_arg("books5", &inst_id);
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let data = json.get("data")?.as_array()?;
            let first = data.first()?;
            let raw: OkxWsBookData = serde_json::from_value(first.clone()).ok()?;
            Some(OrderBook {
                exchange: EXCHANGE,
                symbol: sym,
                bids: parse_levels(&raw.bids),
                asks: parse_levels(&raw.asks),
                timestamp_ms: raw.ts.parse().unwrap_or(0),
                sequence: raw.seq_id,
            })
        }
    })))
}

pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let inst_id = unified_to_okx_swap(symbol);
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

pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let inst_id = unified_to_okx_swap(symbol);
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
// Futures-specific streams
// ---------------------------------------------------------------------------

/// Stream mark price updates for a single symbol.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let inst_id = unified_to_okx_swap(symbol);
    let arg = sub_arg("mark-price", &inst_id);
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: OkxWsMarkPriceData = serde_json::from_value(first.clone()).ok()?;
        Some(raw.into_mark_price(EXCHANGE))
    })))
}

/// Stream liquidation events.
/// OKX liquidation-orders channel uses instType, not instId — subscribes to
/// all SWAP liquidations and filters by symbol.
pub async fn stream_liquidations(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Liquidation>> {
    let arg = sub_arg_inst_type("liquidation-orders", "SWAP");
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, vec![arg]).await?;

    Ok(Box::pin(
        futures::stream::unfold((raw_stream, sym), |(mut stream, sym)| async move {
            loop {
                let json = stream.next().await?;
                let data = json.get("data")?;
                let orders: Vec<OkxWsLiquidationData> =
                    serde_json::from_value(data.clone()).ok()?;
                let liqs: Vec<Liquidation> = orders
                    .into_iter()
                    .filter(|o| {
                        okx_inst_id_to_unified(&o.inst_id)
                            .map(|s| s == sym)
                            .unwrap_or(false)
                    })
                    .flat_map(|o| o.into_liquidations(EXCHANGE))
                    .collect();
                if !liqs.is_empty() {
                    return Some((futures::stream::iter(liqs), (stream, sym)));
                }
            }
        })
        .flatten(),
    ))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("books5", &unified_to_okx_swap(s)))
        .collect();
    let raw_stream = subscribe_and_stream(WS_PUBLIC_URL, args).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let arg = json.get("arg")?;
        let inst_id = arg.get("instId")?.as_str()?;
        let symbol = okx_inst_id_to_unified(inst_id)?;
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;
        let raw: OkxWsBookData = serde_json::from_value(first.clone()).ok()?;
        Some(OrderBook {
            exchange: EXCHANGE,
            symbol,
            bids: parse_levels(&raw.bids),
            asks: parse_levels(&raw.asks),
            timestamp_ms: raw.ts.parse().unwrap_or(0),
            sequence: raw.seq_id,
        })
    })))
}

pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    let args: Vec<serde_json::Value> = symbols
        .iter()
        .map(|s| sub_arg("trades", &unified_to_okx_swap(s)))
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
