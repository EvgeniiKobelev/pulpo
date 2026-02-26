//! WebSocket trade streaming example for KuCoin Spot.
//!
//! Connects to the KuCoin WebSocket and prints the first 20 BTC/USDT trades.
//!
//! Run: cargo run -p gateway-kucoin --example stream_trades

use gateway_core::{Exchange, Symbol};
use gateway_kucoin::KucoinSpot;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let btc = Symbol::new("BTC", "USDT");

    println!("Subscribing to BTC/USDT trades on KuCoin Spot...\n");

    let kucoin = KucoinSpot::public();
    let mut stream = kucoin.stream_trades(&btc).await?;

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
