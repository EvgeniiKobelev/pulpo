use crate::spot::mapper::*;
use crate::spot::rest::KucoinRest;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const EXCHANGE: ExchangeId = ExchangeId::Kucoin;

/// Monotonic counter for unique message IDs.
static MSG_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    MSG_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// WS helpers
// ---------------------------------------------------------------------------

/// Build a KuCoin WS subscription message.
fn make_sub(topic: &str) -> String {
    serde_json::json!({
        "id": next_id(),
        "type": "subscribe",
        "topic": topic,
        "response": true
    })
    .to_string()
}

/// Build a KuCoin WS ping message.
fn make_ping() -> String {
    serde_json::json!({
        "id": next_id(),
        "type": "ping"
    })
    .to_string()
}

/// Obtain a WS endpoint URL with token from the KuCoin bullet-public API.
async fn get_ws_url(config: &ExchangeConfig) -> Result<(String, u64)> {
    let rest = KucoinRest::new(config);
    let bullet = rest.bullet_public().await?;
    let server = bullet.instance_servers.into_iter().next().ok_or_else(|| {
        GatewayError::WebSocket {
            exchange: EXCHANGE,
            message: "no instance servers returned".into(),
        }
    })?;
    let ping_interval_ms = server.ping_interval.unwrap_or(18000);
    // KuCoin requires trailing slash in the path: wss://host/?token=...
    let endpoint = server.endpoint.to_string();
    let url = format!("{}?token={}", endpoint, bullet.token);
    Ok((url, ping_interval_ms))
}

// ---------------------------------------------------------------------------
// Core WS loop
// ---------------------------------------------------------------------------

async fn run_ws_loop(
    config: ExchangeConfig,
    topics: Vec<String>,
    tx: mpsc::Sender<KucoinWsMessage>,
) {
    let mut backoff = Duration::from_secs(1);

    loop {
        if tx.is_closed() {
            break;
        }

        // Get a fresh token + endpoint for each (re)connection.
        let (ws_url, ping_interval_ms) = match get_ws_url(&config).await {
            Ok(v) => {
                backoff = Duration::from_secs(1);
                v
            }
            Err(e) => {
                warn!("KuCoin WS bullet-public failed: {e}, retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };

        let ws = match connect_async(&ws_url).await {
            Ok((ws, _)) => {
                info!("KuCoin WS connected");
                ws
            }
            Err(e) => {
                warn!("KuCoin WS connect failed: {e}, retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };

        let (mut write, mut read) = ws.split();

        // Subscribe to all topics.
        let mut sub_ok = true;
        for topic in &topics {
            if write.send(Message::text(make_sub(topic))).await.is_err() {
                warn!("KuCoin WS subscribe failed for {topic}");
                sub_ok = false;
                break;
            }
        }
        if !sub_ok {
            backoff = (backoff * 2).min(Duration::from_secs(30));
            continue;
        }

        // Ping interval — KuCoin default is 18s, timeout is 10s.
        let ping_secs = (ping_interval_ms / 1000).max(5);
        let mut ping_timer = tokio::time::interval(Duration::from_secs(ping_secs));
        ping_timer.tick().await; // skip first immediate tick

        backoff = Duration::from_secs(1);

        // ---- message loop ----
        loop {
            tokio::select! {
                _ = ping_timer.tick() => {
                    if write.send(Message::text(make_ping())).await.is_err() {
                        warn!("KuCoin WS ping failed");
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(ws_msg) = serde_json::from_str::<KucoinWsMessage>(&text) {
                                // Skip non-message types (pong, ack, welcome)
                                if ws_msg.msg_type != "message" {
                                    continue;
                                }
                                if tx.send(ws_msg).await.is_err() {
                                    debug!("KuCoin WS receiver dropped");
                                    return;
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            let _ = write.send(Message::Pong(data)).await;
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("KuCoin WS closed by server");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("KuCoin WS error: {e}");
                            break;
                        }
                        None => {
                            warn!("KuCoin WS stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        if tx.is_closed() {
            break;
        }
        warn!("KuCoin WS reconnecting in {backoff:?}");
        tokio::time::sleep(backoff).await;
    }
    debug!("KuCoin WS loop ended");
}

/// Spawn WS loop and return a receiver for parsed messages.
fn subscribe_and_stream(
    config: &ExchangeConfig,
    topics: Vec<String>,
) -> mpsc::Receiver<KucoinWsMessage> {
    let (tx, rx) = mpsc::channel::<KucoinWsMessage>(1024);
    let cfg = config.clone();
    tokio::spawn(async move {
        run_ws_loop(cfg, topics, tx).await;
    });
    rx
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream order-book snapshots (depth 5) for a single symbol.
pub async fn stream_orderbook(
    config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let pair = unified_to_kucoin(symbol);
    let topic = format!("/spotMarket/level2Depth5:{pair}");
    let sym = symbol.clone();
    let mut rx = subscribe_and_stream(config, vec![topic]);

    let (tx_out, rx_out) = mpsc::channel::<OrderBook>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Some(data) = msg.data {
                if let Ok(depth) = serde_json::from_value::<KucoinWsDepthData>(data) {
                    let ob = depth.into_orderbook(sym.clone());
                    if tx_out.send(ob).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let pair = unified_to_kucoin(symbol);
    let topic = format!("/market/match:{pair}");
    let mut rx = subscribe_and_stream(config, vec![topic]);

    let (tx_out, rx_out) = mpsc::channel::<Trade>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Some(data) = msg.data {
                if let Ok(trade_data) = serde_json::from_value::<KucoinWsTradeData>(data) {
                    let trade = trade_data.into_trade();
                    if tx_out.send(trade).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

/// Stream kline/candlestick updates for a single symbol.
pub async fn stream_candles(
    config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let pair = unified_to_kucoin(symbol);
    let iv = interval_to_kucoin_ws(interval);
    let topic = format!("/market/candles:{pair}_{iv}");
    let mut rx = subscribe_and_stream(config, vec![topic]);

    let (tx_out, rx_out) = mpsc::channel::<Candle>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Some(data) = msg.data {
                if let Ok(candle_data) = serde_json::from_value::<KucoinWsCandleData>(data) {
                    if let Some(candle) = candle_data.into_candle() {
                        if tx_out.send(candle).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams
// ---------------------------------------------------------------------------

/// Stream order-book snapshots for multiple symbols over a single WS connection.
///
/// KuCoin supports comma-separated symbols in a single topic subscription
/// (up to 100 symbols per topic).
pub async fn stream_orderbooks_batch(
    config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }

    // KuCoin allows up to 100 symbols per topic, so we split into chunks.
    let pairs: Vec<String> = symbols.iter().map(unified_to_kucoin).collect();
    let topics: Vec<String> = pairs
        .chunks(100)
        .map(|chunk| format!("/spotMarket/level2Depth5:{}", chunk.join(",")))
        .collect();
    let mut rx = subscribe_and_stream(config, topics);

    let (tx_out, rx_out) = mpsc::channel::<OrderBook>(256);

    // We need to extract the symbol from the topic for batch streams
    let symbols_owned: Vec<Symbol> = symbols.to_vec();
    let default_sym = symbols_owned
        .first()
        .cloned()
        .unwrap_or_else(|| Symbol::new("", ""));

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            // Extract symbol from topic: /spotMarket/level2Depth5:BTC-USDT
            let sym = msg
                .topic
                .as_deref()
                .and_then(|t| t.split(':').nth(1))
                .map(kucoin_pair_to_unified)
                .unwrap_or_else(|| default_sym.clone());

            if let Some(data) = msg.data {
                if let Ok(depth) = serde_json::from_value::<KucoinWsDepthData>(data) {
                    let ob = depth.into_orderbook(sym);
                    if tx_out.send(ob).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

/// Stream real-time trades for multiple symbols over a single WS connection.
///
/// KuCoin allows up to 100 symbols per topic, so we split into chunks.
pub async fn stream_trades_batch(
    config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }

    let pairs: Vec<String> = symbols.iter().map(unified_to_kucoin).collect();
    let topics: Vec<String> = pairs
        .chunks(100)
        .map(|chunk| format!("/market/match:{}", chunk.join(",")))
        .collect();
    let mut rx = subscribe_and_stream(config, topics);

    let (tx_out, rx_out) = mpsc::channel::<Trade>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Some(data) = msg.data {
                if let Ok(trade_data) = serde_json::from_value::<KucoinWsTradeData>(data) {
                    let trade = trade_data.into_trade();
                    if tx_out.send(trade).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}
