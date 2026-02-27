use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://fx-ws.gateio.ws/v4/ws/usdt";
const EXCHANGE: ExchangeId = ExchangeId::GateFutures;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn make_sub(channel: &str, payload: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "time": now_secs(),
        "channel": channel,
        "event": "subscribe",
        "payload": payload
    })
    .to_string()
}

fn make_ping() -> String {
    serde_json::json!({
        "time": now_secs(),
        "channel": "futures.ping"
    })
    .to_string()
}

/// Connect to the Gate.io futures WebSocket, subscribe to the given channel,
/// and return a [`BoxStream`] that yields parsed JSON `"result"` objects.
///
/// Pong and subscribe confirmations are filtered out. The connection is
/// automatically re-established with exponential back-off.
async fn subscribe_and_stream(
    channel: &str,
    payload: Vec<serde_json::Value>,
) -> Result<BoxStream<serde_json::Value>> {
    let sub_msg = make_sub(channel, payload.clone());
    let channel_owned = channel.to_string();

    let (ws_stream, _) = connect_async(WS_URL)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    write
        .send(Message::text(sub_msg.clone()))
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
            let _ = write.send(Message::text(make_ping())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
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
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("channel").and_then(|v| v.as_str()) == Some("futures.pong") {
                                        continue;
                                    }
                                    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                                    if event == "subscribe" || event == "unsubscribe" {
                                        continue;
                                    }
                                    // futures.order_book uses "all" for snapshots,
                                    // other channels use "update".
                                    if (event == "update" || event == "all") && json.get("result").is_some() {
                                        if tx.send(json).await.is_err() {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Gate Futures WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Gate Futures WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Gate Futures WS stream ended unexpectedly");
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
                warn!("Gate Futures WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let sub = make_sub(&channel_owned, payload.clone());
                        if new_write
                            .send(Message::text(sub))
                            .await
                            .is_err()
                        {
                            warn!("Gate Futures WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Gate Futures WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Gate Futures WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Gate Futures WS stream ended");
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
    let contract = unified_to_gate(symbol);
    let payload = vec![
        serde_json::Value::String(contract),
        serde_json::Value::String("20".into()),
        serde_json::Value::String("0".into()),
    ];
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream("futures.order_book", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let result = json.get("result")?;
            let raw: GateFuturesWsOrderBookResult = serde_json::from_value(result.clone()).ok()?;
            Some(raw.into_orderbook(sym))
        }
    })))
}

pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let contract = unified_to_gate(symbol);
    let payload = vec![serde_json::Value::String(contract)];
    let raw_stream = subscribe_and_stream("futures.trades", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let result = json.get("result")?;
        // futures.trades result can be a single object or an array
        if result.is_array() {
            let items: Vec<GateFuturesWsTradeResult> =
                serde_json::from_value(result.clone()).ok()?;
            items.into_iter().next().map(|t| t.into_trade())
        } else {
            let raw: GateFuturesWsTradeResult = serde_json::from_value(result.clone()).ok()?;
            Some(raw.into_trade())
        }
    })))
}

pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let contract = unified_to_gate(symbol);
    let iv = interval_to_gate_futures(interval);
    let payload = vec![
        serde_json::Value::String(iv.into()),
        serde_json::Value::String(contract),
    ];
    let raw_stream = subscribe_and_stream("futures.candlesticks", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let result = json.get("result")?;
        let raw: GateFuturesWsCandleResult = serde_json::from_value(result.clone()).ok()?;
        raw.into_candle()
    })))
}

/// Stream mark price updates via `futures.tickers` channel.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let contract = unified_to_gate(symbol);
    let payload = vec![serde_json::Value::String(contract)];
    let raw_stream = subscribe_and_stream("futures.tickers", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let result = json.get("result")?;
        let raw: GateFuturesWsTickerResult = serde_json::from_value(result.clone()).ok()?;
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

    let (ws_stream, _) = connect_async(WS_URL)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    for sym in symbols {
        let contract = unified_to_gate(sym);
        let sub = make_sub(
            "futures.order_book",
            vec![
                serde_json::Value::String(contract),
                serde_json::Value::String("20".into()),
                serde_json::Value::String("0".into()),
            ],
        );
        write
            .send(Message::text(sub))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;
    }

    let symbols_owned: Vec<Symbol> = symbols.to_vec();
    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            let _ = write.send(Message::text(make_ping())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
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
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("channel").and_then(|v| v.as_str()) == Some("futures.pong") {
                                        continue;
                                    }
                                    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                                    if event == "subscribe" || event == "unsubscribe" {
                                        continue;
                                    }
                                    if (event == "update" || event == "all") && json.get("result").is_some() {
                                        if tx.send(json).await.is_err() {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                }
            }

            // Reconnect
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Gate Futures WS batch reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut ok = true;
                        for sym in &symbols_owned {
                            let contract = unified_to_gate(sym);
                            let sub = make_sub(
                                "futures.order_book",
                                vec![
                                    serde_json::Value::String(contract),
                                    serde_json::Value::String("20".into()),
                                    serde_json::Value::String("0".into()),
                                ],
                            );
                            if new_write.send(Message::text(sub)).await.is_err() {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("Gate Futures WS batch subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Gate Futures WS batch reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Gate Futures WS batch reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Gate Futures WS batch stream ended");
    });

    let default_sym = symbols.first().cloned().unwrap_or_else(|| Symbol::new("", ""));
    Ok(Box::pin(ReceiverStream::new(rx).filter_map(move |json| {
        let default = default_sym.clone();
        async move {
            let result = json.get("result")?;
            let raw: GateFuturesWsOrderBookResult = serde_json::from_value(result.clone()).ok()?;
            Some(raw.into_orderbook(default))
        }
    })))
}

pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }

    let (ws_stream, _) = connect_async(WS_URL)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

    let (mut write, read) = ws_stream.split();

    for sym in symbols {
        let contract = unified_to_gate(sym);
        let sub = make_sub("futures.trades", vec![serde_json::Value::String(contract)]);
        write
            .send(Message::text(sub))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;
    }

    let symbols_owned: Vec<Symbol> = symbols.to_vec();
    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(async move {
        let mut write = write;
        let mut read = read;
        let mut backoff = Duration::from_secs(1);

        'outer: loop {
            let _ = write.send(Message::text(make_ping())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
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
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("channel").and_then(|v| v.as_str()) == Some("futures.pong") {
                                        continue;
                                    }
                                    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                                    if event == "subscribe" || event == "unsubscribe" {
                                        continue;
                                    }
                                    if (event == "update" || event == "all") && json.get("result").is_some() {
                                        if tx.send(json).await.is_err() {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                }
            }

            // Reconnect
            loop {
                if tx.is_closed() {
                    break 'outer;
                }
                warn!("Gate Futures WS trades batch reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut ok = true;
                        for sym in &symbols_owned {
                            let contract = unified_to_gate(sym);
                            let sub = make_sub(
                                "futures.trades",
                                vec![serde_json::Value::String(contract)],
                            );
                            if new_write.send(Message::text(sub)).await.is_err() {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("Gate Futures WS trades batch subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Gate Futures WS trades batch reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Gate Futures WS trades batch reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Gate Futures WS trades batch stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx).filter_map(|json| async move {
        let result = json.get("result")?;
        if result.is_array() {
            let items: Vec<GateFuturesWsTradeResult> =
                serde_json::from_value(result.clone()).ok()?;
            items.into_iter().next().map(|t| t.into_trade())
        } else {
            let raw: GateFuturesWsTradeResult = serde_json::from_value(result.clone()).ok()?;
            Some(raw.into_trade())
        }
    })))
}
