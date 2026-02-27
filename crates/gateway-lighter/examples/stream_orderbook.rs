//! WebSocket order book streaming example for Lighter DEX.
//!
//! Connects to the Lighter WebSocket and prints order book snapshots
//! for ETH/USDC perpetual. Shows top 5 bids and asks on each update.
//!
//! Run: cargo run -p gateway-lighter --example stream_orderbook

use gateway_core::{Exchange, Symbol};
use gateway_lighter::LighterFutures;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let lighter = LighterFutures::public();
    let eth = Symbol::new("ETH", "USDC");

    println!("Subscribing to ETH/USDC order book on Lighter Futures...\n");

    let mut stream = lighter.stream_orderbook(&eth).await?;
    let mut count = 0;

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
        for lvl in ob.bids.iter().take(5) {
            println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
        }
        println!("  --- bids ---");
        println!();

        if count >= 10 {
            println!("Received 10 updates, exiting.");
            break;
        }
    }

    Ok(())
}
