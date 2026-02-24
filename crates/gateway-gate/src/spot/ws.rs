use crate::spot::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://api.gateio.ws/ws/v4/";
const EXCHANGE: ExchangeId = ExchangeId::Gate;

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Build a Gate.io WS subscription message.
fn make_sub(channel: &str, payload: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "time": now_secs(),
        "channel": channel,
        "event": "subscribe",
        "payload": payload
    })
    .to_string()
}

/// Build a Gate.io WS ping message.
fn make_ping() -> String {
    serde_json::json!({
        "time": now_secs(),
        "channel": "spot.ping"
    })
    .to_string()
}

/// Connect to the Gate.io public spot WebSocket, subscribe to the given
/// channel + payload, and return a [`BoxStream`] that yields parsed JSON
/// `"result"` objects for update events only (ping/pong and subscribe
/// confirmations are filtered).
///
/// The connection is automatically re-established with exponential back-off.
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

    // Send SUBSCRIBE message.
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
            // Send initial ping for this connection.
            let _ = write.send(Message::text(make_ping())).await;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
            ping_interval.tick().await; // skip first tick

            // ---- message read loop with periodic ping ----
            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        if write.send(Message::text(make_ping())).await.is_err() {
                            break; // reconnect
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    // Skip pong responses
                                    if json.get("channel").and_then(|v| v.as_str()) == Some("spot.pong") {
                                        continue;
                                    }
                                    // Skip subscribe/unsubscribe confirmations
                                    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                                    if event == "subscribe" || event == "unsubscribe" {
                                        continue;
                                    }
                                    // Only forward "update" events with a "result" field
                                    if event == "update" && json.get("result").is_some() {
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
                                warn!("Gate WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Gate WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Gate WS stream ended unexpectedly");
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
                warn!("Gate WS reconnecting in {backoff:?}…");
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
                            warn!("Gate WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Gate WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Gate WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Gate WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream order-book snapshots for a single symbol.
///
/// Uses the `spot.order_book` channel with 5 levels and 100ms updates.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let pair = unified_to_gate(symbol);
    let payload = vec![
        serde_json::Value::String(pair),
        serde_json::Value::String("5".into()),
        serde_json::Value::String("100ms".into()),
    ];
    let sym = symbol.clone();
    let raw_stream = subscribe_and_stream("spot.order_book", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let sym = sym.clone();
        async move {
            let result = json.get("result")?;
            let raw: GateWsOrderBookResult = serde_json::from_value(result.clone()).ok()?;
            Some(raw.into_orderbook(sym))
        }
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let pair = unified_to_gate(symbol);
    let payload = vec![serde_json::Value::String(pair)];
    let raw_stream = subscribe_and_stream("spot.trades", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let result = json.get("result")?;
        let raw: GateWsTradeResult = serde_json::from_value(result.clone()).ok()?;
        Some(raw.into_trade())
    })))
}

/// Stream kline/candlestick updates for a single symbol.
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let pair = unified_to_gate(symbol);
    let iv = interval_to_gate_ws(interval);
    let payload = vec![
        serde_json::Value::String(iv.into()),
        serde_json::Value::String(pair),
    ];
    let raw_stream = subscribe_and_stream("spot.candlesticks", payload).await?;

    Ok(Box::pin(raw_stream.filter_map(|json| async move {
        let result = json.get("result")?;
        let raw: GateWsCandleResult = serde_json::from_value(result.clone()).ok()?;
        raw.into_candle()
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream order-book updates for multiple symbols over separate subscriptions
/// on a single WebSocket connection.
///
/// Gate.io requires one subscribe per pair for order_book, so we send multiple
/// subscribe messages over the same connection.
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

    // Subscribe to all symbols
    for sym in symbols {
        let pair = unified_to_gate(sym);
        let sub = make_sub(
            "spot.order_book",
            vec![
                serde_json::Value::String(pair),
                serde_json::Value::String("5".into()),
                serde_json::Value::String("100ms".into()),
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
                                    if json.get("channel").and_then(|v| v.as_str()) == Some("spot.pong") {
                                        continue;
                                    }
                                    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                                    if event == "subscribe" || event == "unsubscribe" {
                                        continue;
                                    }
                                    if event == "update" && json.get("result").is_some() {
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
                warn!("Gate WS batch reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut ok = true;
                        for sym in &symbols_owned {
                            let pair = unified_to_gate(sym);
                            let sub = make_sub(
                                "spot.order_book",
                                vec![
                                    serde_json::Value::String(pair),
                                    serde_json::Value::String("5".into()),
                                    serde_json::Value::String("100ms".into()),
                                ],
                            );
                            if new_write.send(Message::text(sub)).await.is_err() {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("Gate WS batch subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Gate WS batch reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Gate WS batch reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Gate WS batch stream ended");
    });

    let default_sym = symbols.first().cloned().unwrap_or_else(|| Symbol::new("", ""));
    Ok(Box::pin(ReceiverStream::new(rx).filter_map(move |json| {
        let default = default_sym.clone();
        async move {
            let result = json.get("result")?;
            let raw: GateWsOrderBookResult = serde_json::from_value(result.clone()).ok()?;
            Some(raw.into_orderbook(default))
        }
    })))
}

/// Stream real-time trades for multiple symbols over a single WebSocket.
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

    // Subscribe to all symbols
    for sym in symbols {
        let pair = unified_to_gate(sym);
        let sub = make_sub("spot.trades", vec![serde_json::Value::String(pair)]);
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
                                    if json.get("channel").and_then(|v| v.as_str()) == Some("spot.pong") {
                                        continue;
                                    }
                                    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                                    if event == "subscribe" || event == "unsubscribe" {
                                        continue;
                                    }
                                    if event == "update" && json.get("result").is_some() {
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
                warn!("Gate WS trades batch reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match connect_async(WS_URL).await {
                    Ok((ws, _)) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut ok = true;
                        for sym in &symbols_owned {
                            let pair = unified_to_gate(sym);
                            let sub = make_sub(
                                "spot.trades",
                                vec![serde_json::Value::String(pair)],
                            );
                            if new_write.send(Message::text(sub)).await.is_err() {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("Gate WS trades batch subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        info!("Gate WS trades batch reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Gate WS trades batch reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Gate WS trades batch stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx).filter_map(|json| async move {
        let result = json.get("result")?;
        let raw: GateWsTradeResult = serde_json::from_value(result.clone()).ok()?;
        Some(raw.into_trade())
    })))
}
