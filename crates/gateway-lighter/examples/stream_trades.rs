//! WebSocket trade streaming example for Lighter DEX.
//!
//! Connects to the Lighter WebSocket and prints the first 20 ETH/USDC
//! perpetual futures trades.
//!
//! Run: cargo run -p gateway-lighter --example stream_trades

use gateway_core::{Exchange, Symbol};
use gateway_lighter::LighterFutures;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let lighter = LighterFutures::public();
    let eth = Symbol::new("ETH", "USDC");

    println!("Subscribing to ETH/USDC trades on Lighter Futures...\n");

    let mut stream = lighter.stream_trades(&eth).await?;
    let mut count = 0;

    while let Some(trade) = stream.next().await {
        count += 1;
        println!(
            "#{:>2}  {:?}  {} @ {}  (id: {})",
            count,
            trade.side,
            trade.qty,
            trade.price,
            trade.trade_id.as_deref().unwrap_or("-"),
        );

        if count >= 20 {
            println!("\nReceived 20 trades, exiting.");
            break;
        }
    }

    Ok(())
}
