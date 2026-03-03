use crate::futures::mapper::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://stream.toobit.com/quote/ws/v1";

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

/// A subscription is a symbol + topic pair.
#[derive(Clone, Debug)]
struct Subscription {
    symbol: String,
    topic: String,
}

impl Subscription {
    fn to_message(&self) -> String {
        serde_json::json!({
            "symbol": &self.symbol,
            "topic": &self.topic,
            "event": "sub"
        })
        .to_string()
    }
}

/// Connect to Toobit WebSocket, send subscribe messages, and return a
/// [`BoxStream`] that yields parsed JSON values.
///
/// - Handles Toobit heartbeat: `{"ping": N}` -> `{"pong": N}`.
/// - Filters out subscription confirmations.
/// - Auto-reconnects with exponential back-off (1 s -> 30 s).
async fn subscribe_stream(
    subscriptions: Vec<Subscription>,
) -> Result<BoxStream<serde_json::Value>> {
    let (ws_stream, _) =
        connect_async(WS_URL)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;

    let (tx, rx) = mpsc::channel::<serde_json::Value>(1024);

    tokio::spawn(run_ws_loop(ws_stream, subscriptions, tx));

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// The main WS event loop: reads, sends pongs, subscribes, and reconnects.
async fn run_ws_loop(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    subscriptions: Vec<Subscription>,
    tx: mpsc::Sender<serde_json::Value>,
) {
    let (write, read) = ws_stream.split();
    let mut write = write;
    let mut read = read;
    let mut backoff = Duration::from_secs(1);
    // Toobit disconnects after 5 min without ping; send every 25s.
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
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Handle Toobit heartbeat: {"ping": N}
                                if let Some(ping_val) = json.get("ping") {
                                    let pong = serde_json::json!({"pong": ping_val}).to_string();
                                    if write.send(Message::text(pong)).await.is_err() {
                                        warn!("Toobit WS pong send failed");
                                        break;
                                    }
                                    continue;
                                }

                                // Filter out pong responses
                                if json.get("pong").is_some() {
                                    continue;
                                }

                                // Filter out subscription confirmations
                                if let Some(event) = json.get("event").and_then(|e| e.as_str()) {
                                    match event {
                                        "sub" | "cancel" => continue,
                                        _ => {}
                                    }
                                }

                                // Forward data messages
                                if json.get("data").is_some()
                                    || json.get("symbol").is_some()
                                    || json.get("symbolName").is_some()
                                {
                                    if tx.send(json).await.is_err() {
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                warn!("Toobit WS pong send failed");
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("Toobit WS connection closed");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("Toobit WS error: {}", e);
                            break;
                        }
                        None => {
                            warn!("Toobit WS stream ended unexpectedly");
                            break;
                        }
                        _ => {}
                    }
                }
                // ---- send next subscribe message (interleaved with reads) ----
                _ = sub_delay.tick(), if sub_idx < subscriptions.len() => {
                    let msg = subscriptions[sub_idx].to_message();
                    if write.send(Message::text(msg)).await.is_err() {
                        warn!("Toobit WS subscribe send failed at index {}", sub_idx);
                        break;
                    }
                    sub_idx += 1;
                }
                // ---- periodic ping ----
                _ = ping_interval.tick() => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    let ping = serde_json::json!({"ping": ts}).to_string();
                    if write.send(Message::text(ping)).await.is_err() {
                        warn!("Toobit WS ping send failed");
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
            warn!("Toobit WS reconnecting in {backoff:?}...");
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
                        "Toobit WS reconnected, re-subscribing to {} channels",
                        subscriptions.len()
                    );
                    break;
                }
                Err(e) => {
                    warn!("Toobit WS reconnect failed: {}", e);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
    debug!("Toobit WS stream ended");
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream orderbook snapshots for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let toobit_sym = unified_to_toobit(symbol);
    let symbol_clone = symbol.clone();
    let sub = Subscription {
        symbol: toobit_sym,
        topic: "depth".to_string(),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            // Toobit depth WS: { symbol, symbolName, topic: "depth",
            //   data: [{ "s", "t", "v", "b": [[p,q],...], "a": [[p,q],...] }] }
            let data = json.get("data")?;
            let arr = data.as_array()?;
            let first = arr.first()?;
            let inner: ToobitWsDepthInner = serde_json::from_value(first.clone()).ok()?;
            Some(inner.into_orderbook(&symbol))
        }
    })))
}

/// Stream real-time trades for a single symbol.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let toobit_sym = unified_to_toobit(symbol);
    let symbol_clone = symbol.clone();
    let sub = Subscription {
        symbol: toobit_sym,
        topic: "trade".to_string(),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.flat_map(move |json| {
        let symbol = symbol_clone.clone();
        // Toobit trade WS: { symbol, topic: "trade",
        //   data: [{ "v", "t", "p", "q", "m" }, ...] }
        let trades: Vec<Trade> = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        serde_json::from_value::<ToobitWsTradeData>(v.clone())
                            .ok()
                            .and_then(|t| t.into_trade(&symbol))
                    })
                    .collect()
            })
            .unwrap_or_default();
        futures::stream::iter(trades)
    })))
}

/// Stream candle updates for a single symbol.
pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let toobit_sym = unified_to_toobit(symbol);
    let symbol_clone = symbol.clone();
    let topic = interval_to_toobit_ws(interval).to_string();
    let sub = Subscription {
        symbol: toobit_sym,
        topic,
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            let data = json.get("data")?;
            let kline_data: ToobitWsKlineData = serde_json::from_value(data.clone()).ok()?;
            kline_data.into_candle(&symbol, interval)
        }
    })))
}

/// Stream mark price updates for a single symbol.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let toobit_sym = unified_to_toobit(symbol);
    let symbol_clone = symbol.clone();
    let sub = Subscription {
        symbol: toobit_sym,
        topic: "markPrice".to_string(),
    };
    let raw = subscribe_stream(vec![sub]).await?;

    Ok(Box::pin(raw.filter_map(move |json| {
        let symbol = symbol_clone.clone();
        async move {
            let data = json.get("data")?;
            let mp: ToobitWsMarkPriceData = serde_json::from_value(data.clone()).ok()?;
            Some(mp.into_mark_price(&symbol))
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

    let all_subs: Vec<(Subscription, Symbol)> = symbols
        .iter()
        .map(|s| {
            (
                Subscription {
                    symbol: unified_to_toobit(s),
                    topic: "depth".to_string(),
                },
                s.clone(),
            )
        })
        .collect();

    let chunks: Vec<Vec<(Subscription, Symbol)>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "Toobit WS: sharding {} orderbook subs across {} connections (~{} each)",
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
            "Toobit WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let subs: Vec<Subscription> = chunk.iter().map(|(s, _)| s.clone()).collect();
        let sym_map: std::collections::HashMap<String, Symbol> = chunk
            .into_iter()
            .map(|(s, sym)| (s.symbol, sym))
            .collect();

        let raw = subscribe_stream(subs).await?;
        let mapped: BoxStream<OrderBook> = Box::pin(raw.filter_map(move |json| {
            let sym_map = sym_map.clone();
            async move {
                let data = json.get("data")?;
                let arr = data.as_array()?;
                let first = arr.first()?;
                let inner: ToobitWsDepthInner = serde_json::from_value(first.clone()).ok()?;
                let raw_sym = json.get("symbol")?.as_str()?;
                let symbol = sym_map.get(raw_sym)?;
                Some(inner.into_orderbook(symbol))
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

    let all_subs: Vec<(Subscription, Symbol)> = symbols
        .iter()
        .map(|s| {
            (
                Subscription {
                    symbol: unified_to_toobit(s),
                    topic: "trade".to_string(),
                },
                s.clone(),
            )
        })
        .collect();

    let chunks: Vec<Vec<(Subscription, Symbol)>> = all_subs
        .chunks(MAX_SUBS_PER_CONNECTION)
        .map(|c| c.to_vec())
        .collect();

    let n_conns = chunks.len();
    if n_conns > 1 {
        info!(
            "Toobit WS: sharding {} trade subs across {} connections (~{} each)",
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
            "Toobit WS: opening connection {}/{} ({} subs)",
            i + 1,
            n_conns,
            chunk.len()
        );
        let subs: Vec<Subscription> = chunk.iter().map(|(s, _)| s.clone()).collect();
        let sym_map: std::collections::HashMap<String, Symbol> = chunk
            .into_iter()
            .map(|(s, sym)| (s.symbol, sym))
            .collect();

        let raw = subscribe_stream(subs).await?;
        let mapped: BoxStream<Trade> = Box::pin(raw.flat_map(move |json| {
            let sym_map = sym_map.clone();
            let raw_sym = json
                .get("symbol")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let symbol = sym_map
                .get(&raw_sym)
                .cloned()
                .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));

            let trades: Vec<Trade> = json
                .get("data")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            serde_json::from_value::<ToobitWsTradeData>(v.clone())
                                .ok()
                                .and_then(|t| t.into_trade(&symbol))
                        })
                        .collect()
                })
                .unwrap_or_default();
            futures::stream::iter(trades)
        }));
        select_all.push(mapped);
    }

    Ok(Box::pin(select_all))
}
