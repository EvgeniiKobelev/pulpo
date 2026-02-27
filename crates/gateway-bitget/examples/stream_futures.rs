//! WebSocket streaming example for Bitget futures.
//!
//! Demonstrates all futures WebSocket streams:
//!   1. Order book snapshots
//!   2. Real-time trades
//!   3. Candlestick updates
//!   4. Mark price
//!   5. Liquidations
//!   6. Batch orderbook (multi-symbol)
//!   7. Batch trades (multi-symbol)
//!
//! Run: cargo run -p gateway-bitget --example stream_futures

use gateway_bitget::BitgetFutures;
use gateway_core::{Exchange, FuturesExchange, Interval, Symbol};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = BitgetFutures::public();
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
                "  {:?}  {} @ {}  (id: {})",
                trade.side,
                trade.qty,
                trade.price,
                trade.trade_id.as_deref().unwrap_or("-"),
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

    // ── 5. Liquidations Stream ──
    println!("\n=== Futures Liquidations Stream — BTC/USDT (waiting up to 30s for 3 events) ===\n");
    let mut stream = futures.stream_liquidations(&btc).await?;
    let mut count = 0;
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(30));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            Some(liq) = stream.next() => {
                count += 1;
                println!(
                    "  #{count}  {:?}  {} @ {}",
                    liq.side, liq.qty, liq.price,
                );
                if count >= 3 {
                    break;
                }
            }
            _ = &mut timeout => {
                println!("  (timed out after 30s, got {count} events)");
                break;
            }
        }
    }
    drop(stream);

    // ── 6. Batch Orderbook Stream (multi-symbol) ──
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

    // ── 7. Batch Trades Stream (multi-symbol) ──
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
