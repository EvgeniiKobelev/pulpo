//! Raw WebSocket debug — tests MEXC endpoints.
//!
//! Run: cargo run -p gateway-mexc --example ws_debug
//! Old spot: cargo run -p gateway-mexc --example ws_debug -- old
//! Futures: cargo run -p gateway-mexc --example ws_debug -- futures

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_default();

    match arg.as_str() {
        "futures" => {
            test_endpoint(
                "wss://contract.mexc.com/edge",
                "futures",
                vec![
                    serde_json::json!({"method": "sub.deal", "param": {"symbol": "BTC_USDT"}}),
                    serde_json::json!({"method": "sub.ticker", "param": {"symbol": "BTC_USDT"}}),
                ],
                serde_json::json!({"method": "ping"}),
            )
            .await?;
        }
        "old" => {
            test_endpoint(
                "wss://wbs.mexc.com/ws",
                "spot-old (deprecated)",
                vec![serde_json::json!({
                    "method": "SUBSCRIPTION",
                    "params": [
                        "spot@public.deals.v3.api@BTCUSDT",
                        "spot@public.miniTicker.v3.api@BTCUSDT"
                    ]
                })],
                serde_json::json!({"method": "PING"}),
            )
            .await?;
        }
        _ => {
            // New spot v3 with protobuf channels (default)
            test_endpoint(
                "wss://wbs-api.mexc.com/ws",
                "spot-proto",
                vec![serde_json::json!({
                    "method": "SUBSCRIPTION",
                    "params": [
                        "spot@public.aggre.deals.v3.api.pb@100ms@BTCUSDT",
                        "spot@public.aggre.bookTicker.v3.api.pb@100ms@BTCUSDT"
                    ]
                })],
                serde_json::json!({"method": "PING"}),
            )
            .await?;
        }
    }

    Ok(())
}

async fn test_endpoint(
    url: &str,
    mode: &str,
    subs: Vec<serde_json::Value>,
    ping: serde_json::Value,
) -> anyhow::Result<()> {
    println!("Mode: {mode}");
    println!("Connecting to {url}...\n");

    let (ws, _) = connect_async(url).await?;
    println!("Connected!\n");

    let (mut write, mut read) = ws.split();

    for sub in &subs {
        println!(">>> {sub}");
        write.send(Message::text(sub.to_string())).await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    write.send(Message::text(ping.to_string())).await?;

    let mut count = 0;
    let mut data_msgs = 0;

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                count += 1;
                let display = if text.len() > 500 {
                    format!("{}... ({} bytes)", &text[..500], text.len())
                } else {
                    text.to_string()
                };

                if text.contains("Blocked") {
                    println!("  BLOCKED: {display}");
                } else if text.contains("pong") || text.contains("PONG") {
                    println!("  PONG ok");
                } else if text.contains("SUBSCRIPTION") || text.contains("rs.sub") {
                    println!("  SUB CONFIRM: {display}");
                } else {
                    data_msgs += 1;
                    println!("  TEXT [{data_msgs}]: {display}");
                }
            }
            Ok(Message::Binary(data)) => {
                data_msgs += 1;
                let hex: String = data
                    .iter()
                    .take(60)
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("  BINARY [{data_msgs}]: {} bytes | {hex}", data.len());
            }
            Ok(Message::Ping(data)) => {
                write.send(Message::Pong(data)).await?;
            }
            Ok(Message::Close(frame)) => {
                println!("\nServer closed: {frame:?}");
                break;
            }
            Err(e) => {
                println!("\nError: {e}");
                break;
            }
            _ => {}
        }

        if data_msgs >= 15 || count >= 60 {
            break;
        }
    }

    println!("\n--- Summary: {data_msgs} data messages, {count} total ---");
    Ok(())
}
