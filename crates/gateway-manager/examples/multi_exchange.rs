//! Multi-exchange example using GatewayManager.
//!
//! Demonstrates:
//! - Registering multiple exchanges
//! - Parallel ticker fetching across all exchanges
//! - BTC/USDT price comparison between Binance and Bybit
//! - Merged trade streams from multiple exchanges
//!
//! Run: cargo run -p gateway-manager --example multi_exchange

use gateway_binance::Binance;
use gateway_bybit::Bybit;
use gateway_core::{ExchangeId, Symbol};
use gateway_manager::GatewayManager;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ── Setup ──
    let mut manager = GatewayManager::new();
    manager.register(Binance::public());
    manager.register(Bybit::public());

    println!("Registered {} exchanges\n", manager.all().len());

    let btc = Symbol::new("BTC", "USDT");

    // ── Parallel Ticker Fetch ──
    println!("=== All Tickers (parallel fetch) ===");
    let results = manager.all_tickers_everywhere().await;
    for (id, result) in &results {
        match result {
            Ok(tickers) => println!("  {}: {} tickers", id, tickers.len()),
            Err(e) => println!("  {}: error — {}", id, e),
        }
    }
    println!();

    // ── Price Comparison ──
    println!("=== BTC/USDT Price Comparison ===");
    for (id, result) in &results {
        if let Ok(tickers) = result {
            if let Some(tick) = tickers.iter().find(|t| t.symbol == btc) {
                println!("  {}: {}", id, tick.last_price);
            }
        }
    }
    println!();

    // ── Merged Trade Streams ──
    println!("=== Merged Trade Stream (BTC/USDT from both exchanges, first 20) ===\n");
    let pairs = vec![
        (ExchangeId::Binance, btc.clone()),
        (ExchangeId::Bybit, btc.clone()),
    ];

    let mut stream = manager.stream_trades_multi(&pairs).await?;
    let mut count = 0;

    while let Some(trade) = stream.next().await {
        count += 1;
        println!(
            "#{:>2}  [{:>7}]  {:?}  {} @ {}",
            count, trade.exchange, trade.side, trade.qty, trade.price,
        );

        if count >= 20 {
            println!("\nReceived 20 trades, exiting.");
            break;
        }
    }

    Ok(())
}
