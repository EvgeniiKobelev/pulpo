//! Batch WebSocket order book streaming for ALL MEXC futures tickers.
//!
//! 1. Fetches all trading futures symbols via REST.
//! 2. Splits them into chunks (default 20 symbols per WS connection).
//! 3. Subscribes to order book streams in parallel via `stream_orderbooks_batch`.
//! 4. Merges all streams and prints updates, tracking unique symbols seen.
//!
//! This verifies that chunked batch subscriptions work correctly.
//!
//! Run: cargo run -p gateway-mexc --example stream_futures_batch
//! Custom chunk size: cargo run -p gateway-mexc --example stream_futures_batch -- 50

use gateway_core::{Exchange, Symbol};
use gateway_mexc::MexcFutures;
use std::collections::HashMap;
use tokio_stream::StreamExt;

const DEFAULT_CHUNK_SIZE: usize = 20;
const MAX_UPDATES: u32 = 200;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let chunk_size: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CHUNK_SIZE);

    let futures = MexcFutures::public();

    // ── 1. Fetch all trading symbols ──
    println!("Fetching MEXC Futures exchange info...");
    let info = futures.exchange_info().await?;
    let symbols: Vec<Symbol> = info
        .symbols
        .iter()
        .filter(|s| s.status == gateway_core::SymbolStatus::Trading)
        .map(|s| s.symbol.clone())
        .collect();

    println!(
        "Found {} trading symbols. Chunk size: {}. Chunks: {}\n",
        symbols.len(),
        chunk_size,
        (symbols.len() + chunk_size - 1) / chunk_size,
    );

    if symbols.is_empty() {
        println!("No symbols found.");
        return Ok(());
    }

    // ── 2. Split into chunks and subscribe ──
    let chunks: Vec<&[Symbol]> = symbols.chunks(chunk_size).collect();
    let mut all_streams = futures::stream::SelectAll::new();

    for (i, chunk) in chunks.iter().enumerate() {
        println!(
            "  Chunk #{}: {} symbols ({}..{})",
            i + 1,
            chunk.len(),
            chunk.first().map(|s| s.to_string()).unwrap_or_default(),
            chunk.last().map(|s| s.to_string()).unwrap_or_default(),
        );
        let stream = futures.stream_orderbooks_batch(chunk).await?;
        all_streams.push(stream);
    }

    println!(
        "\nAll {} chunks subscribed. Waiting for updates...\n",
        chunks.len()
    );

    // ── 3. Consume merged stream ──
    let mut count = 0u32;
    let mut seen: HashMap<String, u32> = HashMap::new();

    while let Some(ob) = all_streams.next().await {
        count += 1;
        let key = ob.symbol.to_string();
        *seen.entry(key.clone()).or_insert(0) += 1;

        let best_bid = ob
            .best_bid()
            .map(|l| l.price.to_string())
            .unwrap_or_else(|| "-".into());
        let best_ask = ob
            .best_ask()
            .map(|l| l.price.to_string())
            .unwrap_or_else(|| "-".into());

        println!(
            "  #{count:>4}  [{:>12}]  bid: {best_bid:>14}  ask: {best_ask:>14}  levels: {}/{}",
            ob.symbol,
            ob.bids.len(),
            ob.asks.len(),
        );

        if count >= MAX_UPDATES {
            break;
        }
    }

    // ── 4. Summary ──
    println!("\n=== Summary ===");
    println!("Total updates received: {count}");
    println!("Unique symbols seen:    {}", seen.len());
    println!("Total symbols:          {}", symbols.len());
    println!(
        "Coverage:               {:.1}%",
        seen.len() as f64 / symbols.len() as f64 * 100.0,
    );

    println!("\nPer-symbol update counts (top 20):");
    let mut counts: Vec<_> = seen.iter().collect();
    counts.sort_by(|a, b| b.1.cmp(a.1));
    for (sym, cnt) in counts.iter().take(20) {
        println!("  {sym:>12}: {cnt} updates");
    }

    if seen.len() < symbols.len() {
        let missing: Vec<_> = symbols
            .iter()
            .filter(|s| !seen.contains_key(&s.to_string()))
            .take(10)
            .collect();
        println!(
            "\nSymbols with no updates (first 10 of {}):",
            symbols.len() - seen.len()
        );
        for s in &missing {
            println!("  {s}");
        }
    }

    println!("\nDone.");
    Ok(())
}
