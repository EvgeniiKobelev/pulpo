//! WebSocket mark price streaming example for Asterdex Futures.
//!
//! Connects to the Asterdex WebSocket and prints mark/index price updates
//! for BTC/USDT perpetual.
//!
//! Run: cargo run -p gateway-asterdex --example stream_mark_price

use gateway_asterdex::AsterdexFutures;
use gateway_core::{FuturesExchange, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = AsterdexFutures::public();
    let btc = Symbol::new("BTC", "USDT");

    println!("Subscribing to BTC/USDT mark price on Asterdex Futures...\n");

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
