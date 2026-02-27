//! WebSocket streaming example for Gate.io futures.
//!
//! Demonstrates all futures WebSocket streams:
//!   1. Order book snapshots
//!   2. Real-time trades
//!   3. Candlestick updates
//!   4. Mark price via tickers
//!
//! Run: cargo run -p gateway-gate --example stream_futures

use gateway_core::{Exchange, FuturesExchange, Interval, Symbol};
use gateway_gate::GateFutures;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = GateFutures::public();
    let btc = Symbol::new("BTC", "USDT");
    let eth = Symbol::new("ETH", "USDT");

    // ── 1. Order Book Stream ──
    println!("=== Futures Order Book Stream — BTC/USDT (5 updates) ===\n");
    let mut stream = futures.stream_orderbook(&btc).await?;
    for i in 1..=5 {
        if let Some(ob) = stream.next().await {
            let best_bid = ob.best_bid().map(|l| l.price.to_string()).unwrap_or("-".into());
            let best_ask = ob.best_ask().map(|l| l.price.to_string()).unwrap_or("-".into());
            let spread = ob.spread().map(|d| d.to_string()).unwrap_or("-".into());
            println!(
                "  #{i}  bid: {best_bid}  ask: {best_ask}  spread: {spread}  levels: {}/{}",
                ob.bids.len(),
                ob.asks.len(),
            );
        }
    }
    drop(stream);

    // ── 2. Trades Stream ──
    println!("\n=== Futures Trades Stream — BTC/USDT (10 trades) ===\n");
    let mut stream = futures.stream_trades(&btc).await?;
    for _ in 0..10 {
        if let Some(trade) = stream.next().await {
            println!(
                "  {:?}  {} contracts @ {}",
                trade.side, trade.qty, trade.price,
            );
        }
    }
    drop(stream);

    // ── 3. Candles Stream ──
    println!("\n=== Futures Candles Stream — BTC/USDT 1m (3 updates) ===\n");
    let mut stream = futures.stream_candles(&btc, Interval::M1).await?;
    for i in 1..=3 {
        if let Some(candle) = stream.next().await {
            println!(
                "  #{i}  O={} H={} L={} C={} V={}",
                candle.open, candle.high, candle.low, candle.close, candle.volume,
            );
        }
    }
    drop(stream);

    // ── 4. Mark Price Stream ──
    println!("\n=== Futures Mark Price Stream — BTC/USDT (5 updates) ===\n");
    let mut stream = futures.stream_mark_price(&btc).await?;
    for i in 1..=5 {
        if let Some(mp) = stream.next().await {
            println!(
                "  #{i}  mark: {}  index: {}",
                mp.mark_price, mp.index_price,
            );
        }
    }
    drop(stream);

    // ── 5. Batch Orderbook Stream (multi-symbol) ──
    println!("\n=== Futures Batch Order Book — BTC + ETH (10 updates) ===\n");
    let mut stream = futures
        .stream_orderbooks_batch(&[btc.clone(), eth.clone()])
        .await?;
    for i in 1..=10 {
        if let Some(ob) = stream.next().await {
            let best_bid = ob.best_bid().map(|l| l.price.to_string()).unwrap_or("-".into());
            let best_ask = ob.best_ask().map(|l| l.price.to_string()).unwrap_or("-".into());
            println!(
                "  #{i:>2}  [{:>8}]  bid: {best_bid}  ask: {best_ask}",
                ob.symbol,
            );
        }
    }
    drop(stream);

    // ── 6. Batch Trades Stream (multi-symbol) ──
    println!("\n=== Futures Batch Trades — BTC + ETH (10 trades) ===\n");
    let mut stream = futures
        .stream_trades_batch(&[btc, eth])
        .await?;
    for i in 1..=10 {
        if let Some(trade) = stream.next().await {
            println!(
                "  #{i:>2}  [{:>8}]  {:?}  {} @ {}",
                trade.symbol, trade.side, trade.qty, trade.price,
            );
        }
    }

    println!("\nDone.");
    Ok(())
}
