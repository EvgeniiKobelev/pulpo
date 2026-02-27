//! WebSocket trade streaming example for Bitget.
//!
//! Streams BTC/USDT trades from both spot and futures,
//! printing the first 20 trades from each.
//!
//! Run: cargo run -p gateway-bitget --example stream_trades

use gateway_bitget::{BitgetFutures, BitgetSpot};
use gateway_core::{Exchange, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let btc = Symbol::new("BTC", "USDT");

    // ── Spot trades ──
    println!("=== Bitget Spot — BTC/USDT Trades (first 20) ===\n");
    let spot = BitgetSpot::public();
    let mut stream = spot.stream_trades(&btc).await?;
    let mut count = 0;

    while let Some(trade) = stream.next().await {
        count += 1;
        println!(
            "  #{:>2}  {:?}  {} @ {}  (id: {})",
            count,
            trade.side,
            trade.qty,
            trade.price,
            trade.trade_id.as_deref().unwrap_or("-"),
        );
        if count >= 20 {
            break;
        }
    }
    drop(stream);

    // ── Futures trades ──
    println!("\n=== Bitget Futures — BTC/USDT Trades (first 20) ===\n");
    let futures = BitgetFutures::public();
    let mut stream = futures.stream_trades(&btc).await?;
    let mut count = 0;

    while let Some(trade) = stream.next().await {
        count += 1;
        println!(
            "  #{:>2}  {:?}  {} @ {}  (id: {})",
            count,
            trade.side,
            trade.qty,
            trade.price,
            trade.trade_id.as_deref().unwrap_or("-"),
        );
        if count >= 20 {
            break;
        }
    }

    println!("\nDone.");
    Ok(())
}
