//! WebSocket liquidation streaming example for Asterdex Futures.
//!
//! Connects to the Asterdex WebSocket and prints force order (liquidation)
//! events for BTC/USDT perpetual.
//!
//! Run: cargo run -p gateway-asterdex --example stream_liquidations

use gateway_asterdex::AsterdexFutures;
use gateway_core::{FuturesExchange, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = AsterdexFutures::public();
    let btc = Symbol::new("BTC", "USDT");

    println!("Subscribing to BTC/USDT liquidations on Asterdex Futures...");
    println!("(liquidations are infrequent — this may take a while)\n");

    let mut stream = futures.stream_liquidations(&btc).await?;
    let mut count = 0;

    while let Some(liq) = stream.next().await {
        count += 1;
        println!(
            "#{:>2}  {:?}  {} @ {}  ts={}",
            count, liq.side, liq.qty, liq.price, liq.timestamp_ms,
        );

        if count >= 10 {
            println!("\nReceived 10 liquidations, exiting.");
            break;
        }
    }

    Ok(())
}
