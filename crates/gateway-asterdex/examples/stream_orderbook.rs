//! WebSocket order book streaming example for Asterdex Futures.
//!
//! Subscribes to a single symbol depth stream and prints top-5 bids/asks.
//!
//! Run: cargo run -p gateway-asterdex --example stream_orderbook
//! Custom symbol: cargo run -p gateway-asterdex --example stream_orderbook -- ETH/USDT

use gateway_asterdex::AsterdexFutures;
use gateway_core::{Exchange, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let symbol = std::env::args()
        .nth(1)
        .and_then(|s| Symbol::parse(&s))
        .unwrap_or_else(|| Symbol::new("BTC", "USDT"));

    let futures = AsterdexFutures::public();

    println!(
        "Subscribing to {}/{} order book on Asterdex Futures...\n",
        symbol.base, symbol.quote
    );

    let mut stream = futures.stream_orderbook(&symbol).await?;
    let mut count = 0u32;

    while let Some(ob) = stream.next().await {
        count += 1;
        println!("--- Update #{count} ---");
        println!("  {:>14}  {:>14}", "PRICE", "QTY");
        println!("  --- asks ---");
        for lvl in ob.asks.iter().take(5).rev() {
            println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
        }
        if let Some(spread) = ob.spread() {
            println!("  --- spread: {} ---", spread);
        }
        println!("  --- bids ---");
        for lvl in ob.bids.iter().take(5) {
            println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
        }
        println!(
            "  levels: {}/{} seq={:?} ts={}",
            ob.bids.len(),
            ob.asks.len(),
            ob.sequence,
            ob.timestamp_ms,
        );
        println!();

        if count >= 20 {
            println!("Received 20 updates, exiting.");
            break;
        }
    }

    Ok(())
}
