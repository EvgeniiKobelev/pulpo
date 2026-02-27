//! WebSocket trade streaming example for Gate.io.
//!
//! Streams BTC/USDT trades from both spot and futures in parallel,
//! printing the first 20 trades from each.
//!
//! Run: cargo run -p gateway-gate --example stream_trades

use gateway_core::{Exchange, Symbol};
use gateway_gate::{GateFutures, GateSpot};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let btc = Symbol::new("BTC", "USDT");

    // ── Spot trades ──
    println!("=== Gate.io Spot — BTC/USDT Trades (first 20) ===\n");
    let spot = GateSpot::public();
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
    println!("\n=== Gate.io Futures — BTC/USDT Trades (first 20) ===\n");
    let futures = GateFutures::public();
    let mut stream = futures.stream_trades(&btc).await?;
    let mut count = 0;

    while let Some(trade) = stream.next().await {
        count += 1;
        println!(
            "  #{:>2}  {:?}  {} contracts @ {}  (id: {})",
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
