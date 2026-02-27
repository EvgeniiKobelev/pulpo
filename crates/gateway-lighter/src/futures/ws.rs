use crate::futures::mapper::*;
use futures::{stream::SelectAll, SinkExt, StreamExt};
use gateway_core::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://mainnet.zklighter.elliot.ai/stream?readonly=true";
const WS_HOST: &str = "mainnet.zklighter.elliot.ai";
const WS_PORT: u16 = 443;

/// Maximum number of channel subscriptions per WebSocket connection.
/// Lighter allows 100 per connection; we use 50 to stay safely within limits.
const CHUNK_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Connection helpers (proxy-aware)
// ---------------------------------------------------------------------------

/// Build a WebSocket upgrade request with browser-like headers.
fn ws_request() -> tokio_tungstenite::tungstenite::http::Request<()> {
    tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(WS_URL)
        .header("Host", WS_HOST)
        .header("Origin", format!("https://{WS_HOST}"))
        .header("User-Agent", "pulpo-loco/0.1")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .expect("valid WS request")
}

/// Minimal Base64 encoder for proxy auth (avoids extra dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Read proxy URL from environment: `LIGHTER_WS_PROXY` or `HTTPS_PROXY`.
///
/// Supported formats:
/// - `http://user:pass@host:port`
/// - `http://host:port`
/// - `host:port`
fn proxy_url() -> Option<String> {
    std::env::var("LIGHTER_WS_PROXY")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("https_proxy"))
        .ok()
        .filter(|s| !s.is_empty())
}

/// Parse `http://user:pass@host:port` into `(host:port, Some(user:pass))`.
fn parse_proxy(raw: &str) -> (&str, Option<&str>) {
    let stripped = raw
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    if let Some(at) = stripped.find('@') {
        (&stripped[at + 1..], Some(&stripped[..at]))
    } else {
        (stripped, None)
    }
}

/// Establish a TCP connection — directly or via HTTP CONNECT proxy.
async fn tcp_connect() -> Result<TcpStream> {
    if let Some(proxy) = proxy_url() {
        let (addr, auth) = parse_proxy(&proxy);

        debug!("Lighter WS: connecting via proxy {addr}");

        let mut stream =
            TcpStream::connect(addr)
                .await
                .map_err(|e| GatewayError::WebSocket {
                    exchange: ExchangeId::LighterFutures,
                    message: format!("proxy TCP connect failed: {e}"),
                })?;

        // HTTP CONNECT tunnel
        let mut req = format!(
            "CONNECT {WS_HOST}:{WS_PORT} HTTP/1.1\r\nHost: {WS_HOST}:{WS_PORT}\r\n"
        );
        if let Some(credentials) = auth {
            let encoded = base64_encode(credentials.as_bytes());
            req.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
        }
        req.push_str("\r\n");

        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
                message: format!("proxy write: {e}"),
            })?;

        // Read the HTTP response byte-by-byte to avoid over-reading into TLS data.
        let mut response = Vec::with_capacity(256);
        let mut byte = [0u8; 1];
        loop {
            stream
                .read_exact(&mut byte)
                .await
                .map_err(|e| GatewayError::WebSocket {
                    exchange: ExchangeId::LighterFutures,
                    message: format!("proxy read: {e}"),
                })?;
            response.push(byte[0]);
            if response.ends_with(b"\r\n\r\n") {
                break;
            }
            if response.len() > 4096 {
                return Err(GatewayError::WebSocket {
                    exchange: ExchangeId::LighterFutures,
                    message: "proxy response too large".into(),
                });
            }
        }

        let resp = String::from_utf8_lossy(&response);
        if !resp.contains("200") {
            return Err(GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
                message: format!(
                    "proxy CONNECT rejected: {}",
                    resp.lines().next().unwrap_or("unknown")
                ),
            });
        }

        Ok(stream)
    } else {
        TcpStream::connect((WS_HOST, WS_PORT))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
                message: format!("TCP connect: {e}"),
            })
    }
}

/// Full WebSocket connect: TCP (± proxy) → TLS → WS handshake.
async fn ws_connect(
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_native_tls::TlsStream<TcpStream>>,
> {
    let tcp = tcp_connect().await?;

    let tls_cx = native_tls::TlsConnector::new().map_err(|e| {
        GatewayError::WebSocket {
            exchange: ExchangeId::LighterFutures,
            message: format!("TLS init: {e}"),
        }
    })?;
    let tls_cx = tokio_native_tls::TlsConnector::from(tls_cx);

    let tls_stream =
        tls_cx
            .connect(WS_HOST, tcp)
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
                message: format!("TLS handshake: {e}"),
            })?;

    let (ws, _) = tokio_tungstenite::client_async(ws_request(), tls_stream)
        .await
        .map_err(|e| GatewayError::WebSocket {
            exchange: ExchangeId::LighterFutures,
            message: e.to_string(),
        })?;

    Ok(ws)
}

// ---------------------------------------------------------------------------
// Core helper
// ---------------------------------------------------------------------------

/// Connect to the Lighter public WebSocket, subscribe to the given channels,
/// and return a [`BoxStream`] of raw JSON values.
///
/// Lighter subscribes one channel per message:
/// ```json
/// {"type": "subscribe", "channel": "trade/0"}
/// ```
///
/// The connection is automatically re-established with exponential back-off
/// whenever the remote side disconnects.
async fn subscribe_and_stream(
    channels: Vec<String>,
) -> Result<BoxStream<serde_json::Value>> {
    let ws = ws_connect().await?;
    let (mut write, read) = ws.split();

    // Send subscribe for each channel.
    for channel in &channels {
        let sub = serde_json::json!({"type": "subscribe", "channel": channel});
        write
            .send(Message::text(sub.to_string()))
            .await
            .map_err(|e| GatewayError::WebSocket {
                exchange: ExchangeId::LighterFutures,
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
            // ---- message read loop with periodic ping ----
            loop {
                tokio::select! {
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(json) =
                                    serde_json::from_str::<serde_json::Value>(&text)
                                {
                                    if json.get("type").and_then(|v| v.as_str())
                                        == Some("subscribed")
                                    {
                                        continue;
                                    }
                                    if json.get("type").and_then(|v| v.as_str())
                                        == Some("error")
                                    {
                                        warn!(
                                            "Lighter WS error: {}",
                                            json.get("message")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("unknown")
                                        );
                                        continue;
                                    }
                                    if tx.send(json).await.is_err() {
                                        break 'outer;
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) => {
                                warn!("Lighter WS connection closed");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("Lighter WS error: {}", e);
                                break;
                            }
                            None => {
                                warn!("Lighter WS stream ended unexpectedly");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = ping_interval.tick() => {
                        if write.send(Message::Ping(vec![].into())).await.is_err() {
                            warn!("Lighter WS ping send failed");
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
                warn!("Lighter WS reconnecting in {backoff:?}…");
                tokio::time::sleep(backoff).await;
                match ws_connect().await {
                    Ok(ws) => {
                        let (mut new_write, new_read) = ws.split();
                        let mut ok = true;
                        for channel in &channels {
                            let sub = serde_json::json!({
                                "type": "subscribe",
                                "channel": channel
                            });
                            if new_write
                                .send(Message::text(sub.to_string()))
                                .await
                                .is_err()
                            {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            warn!("Lighter WS subscribe failed after reconnect");
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                            continue;
                        }
                        write = new_write;
                        read = new_read;
                        backoff = Duration::from_secs(1);
                        ping_interval.reset();
                        info!("Lighter WS reconnected");
                        break;
                    }
                    Err(e) => {
                        warn!("Lighter WS reconnect failed: {}", e);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        debug!("Lighter WS stream ended");
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

/// Stream order-book updates for a single symbol.
pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    market_id: u16,
    symbol: Symbol,
) -> Result<BoxStream<OrderBook>> {
    let channel = format!("order_book/{}", market_id);
    let raw_stream = subscribe_and_stream(vec![channel]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let symbol = symbol.clone();
        async move {
            let ob_data = json.get("order_book")?;
            let raw: LighterWsOrderBookUpdate =
                serde_json::from_value(ob_data.clone()).ok()?;
            Some(raw.into_orderbook(symbol))
        }
    })))
}

/// Stream real-time trades for a single symbol.
///
/// Lighter sends an array of trades per message, so we flatten them.
pub async fn stream_trades(
    _config: &ExchangeConfig,
    market_id: u16,
    symbol: Symbol,
) -> Result<BoxStream<Trade>> {
    let channel = format!("trade/{}", market_id);
    let raw_stream = subscribe_and_stream(vec![channel]).await?;

    Ok(Box::pin(
        futures::stream::unfold(
            (raw_stream, symbol),
            |(mut stream, symbol)| async move {
                loop {
                    let json = stream.next().await?;
                    let Some(trades_data) = json.get("trades") else {
                        continue;
                    };
                    let Ok(trades) =
                        serde_json::from_value::<Vec<LighterWsTrade>>(trades_data.clone())
                    else {
                        continue;
                    };
                    if !trades.is_empty() {
                        let converted: Vec<Trade> = trades
                            .into_iter()
                            .map(|t| t.into_trade(symbol.clone()))
                            .collect();
                        return Some((
                            futures::stream::iter(converted),
                            (stream, symbol),
                        ));
                    }
                }
            },
        )
        .flatten(),
    ))
}

/// Stream mark price updates via the market_stats channel.
pub async fn stream_mark_price(
    _config: &ExchangeConfig,
    market_id: u16,
    symbol: Symbol,
) -> Result<BoxStream<MarkPrice>> {
    let channel = format!("market_stats/{}", market_id);
    let raw_stream = subscribe_and_stream(vec![channel]).await?;

    Ok(Box::pin(raw_stream.filter_map(move |json| {
        let symbol = symbol.clone();
        async move {
            let stats_data = json.get("market_stats")?;
            let raw: LighterWsMarketStats =
                serde_json::from_value(stats_data.clone()).ok()?;
            Some(raw.into_mark_price(symbol))
        }
    })))
}

// ---------------------------------------------------------------------------
// Batch (multi-symbol) streams – chunked across connections
// ---------------------------------------------------------------------------

/// Stream order-book updates for multiple symbols.
///
/// Subscriptions are chunked at [CHUNK_SIZE] channels per WebSocket
/// connection, and the resulting streams are merged via [`SelectAll`].
pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    market_ids: &[(u16, Symbol)],
) -> Result<BoxStream<OrderBook>> {
    let mut all = SelectAll::new();

    for chunk in market_ids.chunks(CHUNK_SIZE) {
        let channels: Vec<String> = chunk
            .iter()
            .map(|(mid, _)| format!("order_book/{}", mid))
            .collect();

        // Build a lookup map for this chunk: market_id → Symbol
        let id_to_symbol: std::collections::HashMap<u16, Symbol> = chunk
            .iter()
            .map(|(mid, sym)| (*mid, sym.clone()))
            .collect();

        let raw_stream = subscribe_and_stream(channels).await?;

        let chunk_stream: BoxStream<OrderBook> =
            Box::pin(raw_stream.filter_map(move |json| {
                let id_to_symbol = id_to_symbol.clone();
                async move {
                    let channel = json.get("channel")?.as_str()?;
                    let market_id = parse_market_id_from_channel(channel)?;
                    let symbol = id_to_symbol.get(&market_id)?.clone();
                    let ob_data = json.get("order_book")?;
                    let raw: LighterWsOrderBookUpdate =
                        serde_json::from_value(ob_data.clone()).ok()?;
                    Some(raw.into_orderbook(symbol))
                }
            }));

        all.push(chunk_stream);
    }

    Ok(Box::pin(all))
}

/// Stream real-time trades for multiple symbols.
///
/// Subscriptions are chunked at [CHUNK_SIZE] channels per WebSocket
/// connection, and the resulting streams are merged via [`SelectAll`].
pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    market_ids: &[(u16, Symbol)],
) -> Result<BoxStream<Trade>> {
    let mut all = SelectAll::new();

    for chunk in market_ids.chunks(CHUNK_SIZE) {
        let channels: Vec<String> = chunk
            .iter()
            .map(|(mid, _)| format!("trade/{}", mid))
            .collect();

        let id_to_symbol: std::collections::HashMap<u16, Symbol> = chunk
            .iter()
            .map(|(mid, sym)| (*mid, sym.clone()))
            .collect();

        let raw_stream = subscribe_and_stream(channels).await?;

        let chunk_stream: BoxStream<Trade> = Box::pin(
            futures::stream::unfold(
                (raw_stream, id_to_symbol),
                |(mut stream, id_to_symbol)| async move {
                    loop {
                        let json = stream.next().await?;
                        let Some(channel) =
                            json.get("channel").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        let Some(market_id) = parse_market_id_from_channel(channel)
                        else {
                            continue;
                        };
                        let Some(symbol) = id_to_symbol.get(&market_id).cloned()
                        else {
                            continue;
                        };
                        let Some(trades_data) = json.get("trades") else {
                            continue;
                        };
                        let Ok(trades) = serde_json::from_value::<Vec<LighterWsTrade>>(
                            trades_data.clone(),
                        ) else {
                            continue;
                        };
                        if !trades.is_empty() {
                            let converted: Vec<Trade> = trades
                                .into_iter()
                                .map(|t| t.into_trade(symbol.clone()))
                                .collect();
                            return Some((
                                futures::stream::iter(converted),
                                (stream, id_to_symbol),
                            ));
                        }
                    }
                },
            )
            .flatten(),
        );

        all.push(chunk_stream);
    }

    Ok(Box::pin(all))
}
