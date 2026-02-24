//! WebSocket trade streaming example for MEXC spot.
//!
//! Run: cargo run -p gateway-mexc --example stream_trades
//! Custom symbol: cargo run -p gateway-mexc --example stream_trades -- ETH/USDT

use gateway_core::{Exchange, Symbol};
use gateway_mexc::MexcSpot;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let symbol = std::env::args()
        .nth(1)
        .and_then(|s| Symbol::parse(&s))
        .unwrap_or_else(|| Symbol::new("BTC", "USDT"));

    println!(
        "Subscribing to {}/{} trades on MEXC spot...\n",
        symbol.base, symbol.quote
    );

    let exchange = MexcSpot::public();
    let mut stream = exchange.stream_trades(&symbol).await?;

    println!("Stream opened. Waiting for trades...\n");

    let mut count = 0;

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

    if count == 0 {
        println!("No trades received.");
    }

    Ok(())
}
