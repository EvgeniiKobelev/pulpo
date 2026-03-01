use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://contract.mexc.com/edge";
const EXCHANGE: ExchangeId = ExchangeId::MexcFutures;

// ---------------------------------------------------------------------------
// Core helpers
// ---------------------------------------------------------------------------

fn make_sub(method: &str, param: serde_json::Value) -> String {
    serde_json::json!({
        "method": method,
        "param": param
    })
    .to_string()
}

fn make_ping() -> String {
    serde_json::json!({ "method": "ping" }).to_string()
}

/// Connect to MEXC futures WS, subscribe, and return a stream of parsed messages.
///
/// Reconnects automatically with exponential back-off.
async fn subscribe_and_stream(
    subs: Vec<String>,
) -> Result<BoxStream<MexcFuturesWsMessage>> {
    let (ws_stream, _) = connect_async(WS_URL)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    // Send all subscription messages
    for sub in &subs {
        write
            .send(Message::text(sub.clone()))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;
    }

    let (tx, rx) = mpsc::channel::<MexcFuturesWsMessage>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            let _ = write.send(Message::text(make_ping())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
            ping_interval.tick().await;

            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text(make_ping())).await.is_err() {
                            break;
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                // Skip pong messages
                                if text.contains("\"channel\":\"pong\"") {
                                    continue;
                                }
                                // Skip subscription confirmations
                                if text.contains("\"channel\":\"rs.") {
                                    continue;
                                }

                                // Parse push messages
                                if let Ok(ws_msg) = serde_json::from_str::<MexcFuturesWsMessage>(&text) {
                                    if ws_msg.channel.starts_with("push.") {
                                        if tx.send(ws_msg).await.is_err() {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("MEXC Futures WS closed by server");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("MEXC Futures WS error: {e}");
                                break;
                            }
                            None => {
                                warn!("MEXC Futures WS stream ended");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Reconnect with exponential back-off
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("MEXC Futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut ok = true;
                        for sub in &subs {
                            if new_write.send(Message::text(sub.clone())).await.is_err() {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("MEXC Futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("MEXC Futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("MEXC Futures WS reconnect failed: {e}");
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("MEXC Futures WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let pair = unified_to_mexc_futures(symbol);
    let sub = make_sub(
        "sub.depth.full",
        serde_json::json!({ "symbol": pair, "limit": 20 }),
    );
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![sub]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |ws_msg| {
        let sym = sym.clone();
        async move {
            if ws_msg.channel != "push.depth.full" {
                return None;
            }
            let raw: MexcFuturesWsDepthFull =
                serde_json::from_value(ws_msg.data).ok()?;
            let ts = ws_msg.ts.unwrap_or(0);
            Some(raw.into_orderbook(sym, ts))
        }
    })))
}

pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let pair = unified_to_mexc_futures(symbol);
    let sub = make_sub(
        "sub.deal",
        serde_json::json!({ "symbol": pair }),
    );
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![sub]).await?;

    let (tx_out, rx_out) = mpsc::channel::<Trade>(256);
    tokio::spawn(async move {
        let mut stream = std::pin::pin!(raw_stream);
        while let Some(ws_msg) = stream.next().await {
            if ws_msg.channel != "push.deal" {
                continue;
            }
            // data can be a single deal or an array of deals
            if ws_msg.data.is_array() {
                if let Ok(deals) = serde_json::from_value::<Vec<MexcFuturesWsDeal>>(ws_msg.data) {
                    for deal in deals {
                        let trade = deal.into_trade(sym.clone());
                        if tx_out.send(trade).await.is_err() {
                            return;
                        }
                    }
                }
            } else if let Ok(deal) = serde_json::from_value::<MexcFuturesWsDeal>(ws_msg.data) {
                let trade = deal.into_trade(sym.clone());
                if tx_out.send(trade).await.is_err() {
                    return;
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let pair = unified_to_mexc_futures(symbol);
    let iv = interval_to_mexc_futures(interval);
    let sub = make_sub(
        "sub.kline",
        serde_json::json!({ "symbol": pair, "interval": iv }),
    );
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream(vec![sub]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |ws_msg| {
        let sym = sym.clone();
        async move {
            if ws_msg.channel != "push.kline" {
                return None;
            }
            let raw: MexcFuturesWsKline =
                serde_json::from_value(ws_msg.data).ok()?;
            raw.into_candle(sym)
        }
    })))
}

/// Stream mark price updates via `sub.ticker` channel.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let pair = unified_to_mexc_futures(symbol);
    let sub = make_sub(
        "sub.ticker",
        serde_json::json!({ "symbol": pair }),
    );
    let raw_stream = subscribe_and_stream(vec![sub]).await?;

    Ok(Box::pin(raw_stream.filter_map(|ws_msg| async move {
        if ws_msg.channel != "push.ticker" {
            return None;
        }
        let raw: MexcFuturesWsTicker =
            serde_json::from_value(ws_msg.data).ok()?;
        raw.into_mark_price()
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }

    let subs: Vec<String> = symbols
        .iter()
        .map(|sym| {
            let pair = unified_to_mexc_futures(sym);
            make_sub(
                "sub.depth.full",
                serde_json::json!({ "symbol": pair, "limit": 20 }),
            )
        })
        .collect();

    let raw_stream = subscribe_and_stream(subs).await?;

    Ok(Box::pin(raw_stream.filter_map(|ws_msg| async move {
        if ws_msg.channel != "push.depth.full" {
            return None;
        }
        let sym = ws_msg
            .symbol
            .as_deref()
            .map(mexc_futures_to_unified)
            .unwrap_or_else(|| Symbol::new("", ""));
        let raw: MexcFuturesWsDepthFull =
            serde_json::from_value(ws_msg.data).ok()?;
        let ts = ws_msg.ts.unwrap_or(0);
        Some(raw.into_orderbook(sym, ts))
    })))
}

pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }

    let subs: Vec<String> = symbols
        .iter()
        .map(|sym| {
            let pair = unified_to_mexc_futures(sym);
            make_sub("sub.deal", serde_json::json!({ "symbol": pair }))
        })
        .collect();

    let raw_stream = subscribe_and_stream(subs).await?;

    let (tx_out, rx_out) = mpsc::channel::<Trade>(256);
    tokio::spawn(async move {
        let mut stream = std::pin::pin!(raw_stream);
        while let Some(ws_msg) = stream.next().await {
            if ws_msg.channel != "push.deal" {
                continue;
            }
            let sym = ws_msg
                .symbol
                .as_deref()
                .map(mexc_futures_to_unified)
                .unwrap_or_else(|| Symbol::new("", ""));
            if ws_msg.data.is_array() {
                if let Ok(deals) = serde_json::from_value::<Vec<MexcFuturesWsDeal>>(ws_msg.data) {
                    for deal in deals {
                        let trade = deal.into_trade(sym.clone());
                        if tx_out.send(trade).await.is_err() {
                            return;
                        }
                    }
                }
            } else if let Ok(deal) = serde_json::from_value::<MexcFuturesWsDeal>(ws_msg.data) {
                let trade = deal.into_trade(sym.clone());
                if tx_out.send(trade).await.is_err() {
                    return;
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}
