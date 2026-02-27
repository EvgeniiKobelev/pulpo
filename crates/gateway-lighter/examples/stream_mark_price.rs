//! WebSocket mark price streaming example for Lighter DEX.
//!
//! Connects to the Lighter WebSocket market_stats channel and prints
//! mark/index price updates for ETH/USDC perpetual.
//!
//! Run: cargo run -p gateway-lighter --example stream_mark_price

use gateway_core::{FuturesExchange, Symbol};
use gateway_lighter::LighterFutures;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let lighter = LighterFutures::public();
    let eth = Symbol::new("ETH", "USDC");

    println!("Subscribing to ETH/USDC mark price on Lighter Futures...\n");

    let mut stream = lighter.stream_mark_price(&eth).await?;
    let mut count = 0;

    while let Some(mp) = stream.next().await {
        count += 1;
        println!(
            "#{:>2}  mark={:<12}  index={:<12}",
            count, mp.mark_price, mp.index_price,
        );

        if count >= 20 {
            println!("\nReceived 20 updates, exiting.");
            break;
        }
    }

    Ok(())
}
