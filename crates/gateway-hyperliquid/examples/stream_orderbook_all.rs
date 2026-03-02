//! Subscribe to order book updates for ALL Hyperliquid perpetual symbols.
//!
//! Fetches the full symbol list via REST, then opens a batch WS subscription.
//! Prints per-symbol update counts every 5 seconds so you can verify that
//! every symbol is receiving data.
//!
//! Run: cargo run -p gateway-hyperliquid --example stream_orderbook_all

use gateway_hyperliquid::HyperliquidFutures;
use gateway_core::{Exchange, Symbol};
use std::collections::HashMap;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = HyperliquidFutures::public();

    // 1. Fetch all available symbols.
    let info = futures.exchange_info().await?;
    let symbols: Vec<Symbol> = info.symbols.into_iter().map(|s| s.symbol).collect();
    println!("Fetched {} symbols from Hyperliquid", symbols.len());
    println!(
        "First 10: {:?}",
        symbols.iter().take(10).map(|s| &s.base).collect::<Vec<_>>()
    );
    println!();

    // 2. Subscribe to ALL orderbooks via batch method.
    println!("Subscribing to {} orderbook streams...", symbols.len());
    let mut stream = futures.stream_orderbooks_batch(&symbols).await?;
    println!("Subscribed! Waiting for data...\n");

    // 3. Track per-symbol update counts.
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total: u64 = 0;
    let mut last_report = tokio::time::Instant::now();
    let report_interval = tokio::time::Duration::from_secs(5);

    while let Some(ob) = stream.next().await {
        *counts.entry(ob.symbol.base.clone()).or_default() += 1;
        total += 1;

        // Print a summary every 5 seconds.
        if last_report.elapsed() >= report_interval {
            let active = counts.len();
            let min = counts.values().min().copied().unwrap_or(0);
            let max = counts.values().max().copied().unwrap_or(0);

            println!("--- Stats after {total} total updates ---");
            println!(
                "  Symbols with data: {active}/{} ({:.0}%)",
                symbols.len(),
                active as f64 / symbols.len() as f64 * 100.0,
            );
            println!("  Updates per symbol: min={min}, max={max}");

            // Show symbols that have NOT received any data yet.
            let missing: Vec<&str> = symbols
                .iter()
                .filter(|s| !counts.contains_key(&s.base))
                .map(|s| s.base.as_str())
                .collect();
            if !missing.is_empty() {
                println!(
                    "  Missing ({}):{} {}",
                    missing.len(),
                    if missing.len() > 20 { " (first 20)" } else { "" },
                    missing.iter().take(20).copied().collect::<Vec<_>>().join(", "),
                );
            }

            // Top-5 most active symbols.
            let mut top: Vec<(&String, &u64)> = counts.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1));
            let top5: Vec<String> = top
                .iter()
                .take(5)
                .map(|(s, c)| format!("{}={}", s, c))
                .collect();
            println!("  Top 5: {}", top5.join(", "));
            println!();

            last_report = tokio::time::Instant::now();

            // Stop after all symbols received at least one update, or after 60s.
            if active == symbols.len() {
                println!("All {} symbols received data!", symbols.len());
                break;
            }
        }
    }

    println!("Done. Total updates: {total}, symbols with data: {}", counts.len());

    Ok(())
}
