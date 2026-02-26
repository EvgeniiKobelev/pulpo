//! Raw WebSocket debug — tests KuCoin WS connection step by step.
//!
//! Run: cargo run -p gateway-kucoin --example ws_debug
//!
//! Tests:
//! 1. POST /api/v1/bullet-public to get token + endpoint
//! 2. Connect to WS with token
//! 3. Subscribe to /market/match:BTC-USDT (trades)
//! 4. Print ALL raw messages received

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const BASE_URL: &str = "https://api.kucoin.com";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ---- Step 1: Get bullet-public token ----
    println!("=== Step 1: POST /api/v1/bullet-public ===\n");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{BASE_URL}/api/v1/bullet-public"))
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;
    println!("HTTP {status}");
    println!("Response: {body}\n");

    let json: serde_json::Value = serde_json::from_str(&body)?;

    let code = json["code"].as_str().unwrap_or("?");
    if code != "200000" {
        println!("ERROR: API returned code={code}, aborting.");
        return Ok(());
    }

    let token = json["data"]["token"].as_str().unwrap_or("");
    let endpoint = json["data"]["instanceServers"][0]["endpoint"]
        .as_str()
        .unwrap_or("");
    let ping_interval = json["data"]["instanceServers"][0]["pingInterval"]
        .as_u64()
        .unwrap_or(18000);

    println!("Token: {}...{}", &token[..20.min(token.len())], &token[token.len().saturating_sub(10)..]);
    println!("Endpoint: {endpoint}");
    println!("Ping interval: {ping_interval}ms\n");

    if token.is_empty() || endpoint.is_empty() {
        println!("ERROR: empty token or endpoint, aborting.");
        return Ok(());
    }

    // ---- Step 2: Connect to WS ----
    println!("=== Step 2: Connect to WebSocket ===\n");

    // KuCoin requires the trailing slash: wss://ws-api-spot.kucoin.com/?token=...
    let ws_url = format!(
        "{}?token={}",
        endpoint,
        token
    );
    println!("Connecting to: {}...{}", &ws_url[..60.min(ws_url.len())], "[truncated]");

    let (ws, _) = connect_async(&ws_url).await?;
    println!("Connected!\n");

    let (mut write, mut read) = ws.split();

    // Read welcome message
    println!("=== Waiting for welcome message ===\n");
    if let Some(msg) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read.next(),
    )
    .await?
    {
        match msg {
            Ok(Message::Text(text)) => println!("WELCOME: {text}\n"),
            other => println!("WELCOME (unexpected): {other:?}\n"),
        }
    }

    // ---- Step 3: Subscribe to trades ----
    println!("=== Step 3: Subscribe to /market/match:BTC-USDT ===\n");

    let sub_msg = serde_json::json!({
        "id": 1,
        "type": "subscribe",
        "topic": "/market/match:BTC-USDT",
        "response": true
    });
    println!(">>> {sub_msg}");
    write
        .send(Message::text(sub_msg.to_string()))
        .await?;

    // Also subscribe to ticker for comparison
    let sub_ticker = serde_json::json!({
        "id": 2,
        "type": "subscribe",
        "topic": "/market/ticker:BTC-USDT",
        "response": true
    });
    println!(">>> {sub_ticker}");
    write
        .send(Message::text(sub_ticker.to_string()))
        .await?;

    // Also subscribe to depth for comparison
    let sub_depth = serde_json::json!({
        "id": 3,
        "type": "subscribe",
        "topic": "/spotMarket/level2Depth5:BTC-USDT",
        "response": true
    });
    println!(">>> {sub_depth}");
    write
        .send(Message::text(sub_depth.to_string()))
        .await?;

    // ---- Step 4: Read messages ----
    println!("\n=== Step 4: Reading messages (30s timeout) ===\n");

    let mut count = 0;
    let mut trade_count = 0;
    let mut ticker_count = 0;
    let mut depth_count = 0;
    let mut other_count = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    let ping_msg = serde_json::json!({
        "id": 99,
        "type": "ping"
    });

    let mut ping_timer =
        tokio::time::interval(std::time::Duration::from_millis(ping_interval / 2));
    ping_timer.tick().await;

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                println!("\n--- 30s timeout reached ---");
                break;
            }
            _ = ping_timer.tick() => {
                write.send(Message::text(ping_msg.to_string())).await?;
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        count += 1;

                        let display = if text.len() > 400 {
                            format!("{}... ({} bytes)", &text[..400], text.len())
                        } else {
                            text.to_string()
                        };

                        if text.contains("\"type\":\"pong\"") {
                            // skip pong
                        } else if text.contains("\"type\":\"ack\"") {
                            println!("  ACK: {display}");
                        } else if text.contains("/market/match") {
                            trade_count += 1;
                            println!("  TRADE [{trade_count}]: {display}");
                        } else if text.contains("/market/ticker") {
                            ticker_count += 1;
                            if ticker_count <= 3 {
                                println!("  TICKER [{ticker_count}]: {display}");
                            }
                        } else if text.contains("level2Depth") {
                            depth_count += 1;
                            if depth_count <= 3 {
                                println!("  DEPTH [{depth_count}]: {display}");
                            }
                        } else {
                            other_count += 1;
                            println!("  OTHER [{other_count}]: {display}");
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        count += 1;
                        println!("  BINARY: {} bytes", data.len());
                    }
                    Some(Ok(Message::Ping(data))) => {
                        write.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        println!("\nServer closed: {frame:?}");
                        break;
                    }
                    Some(Err(e)) => {
                        println!("\nError: {e}");
                        break;
                    }
                    None => {
                        println!("\nStream ended");
                        break;
                    }
                    _ => {}
                }

                // Stop after enough data
                if trade_count >= 10 {
                    println!("\n--- Got 10 trades, stopping early ---");
                    break;
                }
            }
        }
    }

    println!("\n=== Summary ===");
    println!("Total messages: {count}");
    println!("Trades (/market/match): {trade_count}");
    println!("Tickers (/market/ticker): {ticker_count}");
    println!("Depth (level2Depth5): {depth_count}");
    println!("Other: {other_count}");

    if trade_count == 0 {
        println!("\n!!! NO TRADES RECEIVED !!!");
        println!("Possible issues:");
        println!("  - Topic format may be wrong");
        println!("  - KuCoin may have changed the API");
        println!("  - Region/IP restrictions");
    }

    Ok(())
}
