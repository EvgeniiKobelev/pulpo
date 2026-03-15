//! WebSocket trades streaming example for KuCoin Futures.
//!
//! Subscribes to a single symbol trade stream and prints each trade.
//!
//! Run: cargo run -p gateway-kucoin --example futures_stream_trades
//! Custom symbol: cargo run -p gateway-kucoin --example futures_stream_trades -- ETH/USDT

use gateway_core::{Exchange, Symbol};
use gateway_kucoin::KucoinFutures;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let symbol = std::env::args()
        .nth(1)
        .and_then(|s| Symbol::parse(&s))
        .unwrap_or_else(|| Symbol::new("BTC", "USDT"));

    let futures = KucoinFutures::public();

    println!(
        "Subscribing to {}/{} trades on KuCoin Futures...\n",
        symbol.base, symbol.quote
    );

    let mut stream = futures.stream_trades(&symbol).await?;
    let mut count = 0u32;

    while let Some(trade) = stream.next().await {
        count += 1;
        println!(
            "#{:>4}  {:?}  {} @ {}  ts={}",
            count, trade.side, trade.qty, trade.price, trade.timestamp_ms,
        );

        if count >= 50 {
            println!("\nReceived 50 trades, exiting.");
            break;
        }
    }

    Ok(())
}
