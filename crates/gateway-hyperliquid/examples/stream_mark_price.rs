//! WebSocket mark price streaming example for Hyperliquid Futures.
//!
//! Connects to the Hyperliquid WebSocket and prints mark/oracle price updates
//! for BTC/USDC perpetual.
//!
//! Run: cargo run -p gateway-hyperliquid --example stream_mark_price

use gateway_hyperliquid::HyperliquidFutures;
use gateway_core::{FuturesExchange, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = HyperliquidFutures::public();
    let btc = Symbol::new("BTC", "USDC");

    println!("Subscribing to BTC/USDC mark price on Hyperliquid...\n");

    let mut stream = futures.stream_mark_price(&btc).await?;
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
