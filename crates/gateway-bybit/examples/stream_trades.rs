//! WebSocket trade streaming example for Bybit (spot & futures).
//!
//! Connects to the Bybit WebSocket and prints the first 20 BTC/USDT trades.
//!
//! Run spot:    cargo run -p gateway-bybit --example stream_trades
//! Run futures: cargo run -p gateway-bybit --example stream_trades -- futures

use gateway_bybit::{Bybit, BybitFutures};
use gateway_core::{Exchange, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures_mode = std::env::args().nth(1).is_some_and(|a| a == "futures");
    let btc = Symbol::new("BTC", "USDT");
    let market = if futures_mode { "futures" } else { "spot" };

    println!("Subscribing to BTC/USDT trades on Bybit ({market})...\n");

    let mut stream = if futures_mode {
        BybitFutures::public().stream_trades(&btc).await?
    } else {
        Bybit::public().stream_trades(&btc).await?
    };

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
