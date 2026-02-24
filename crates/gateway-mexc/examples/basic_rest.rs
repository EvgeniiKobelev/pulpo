//! Basic REST API example for MEXC spot.
//!
//! Run: cargo run -p gateway-mexc --example basic_rest

use gateway_core::{Exchange, Interval, Symbol};
use gateway_mexc::MexcSpot;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let exchange = MexcSpot::public();
    let btc = Symbol::new("BTC", "USDT");

    // Ticker
    println!("=== Ticker BTC/USDT ===");
    let ticker = exchange.ticker(&btc).await?;
    println!(
        "  Last: {}  Bid: {:?}  Ask: {:?}  Vol: {}",
        ticker.last_price, ticker.bid, ticker.ask, ticker.volume_24h
    );

    // Order book
    println!("\n=== OrderBook BTC/USDT (top 5) ===");
    let ob = exchange.orderbook(&btc, 5).await?;
    for ask in ob.asks.iter().rev() {
        println!("  ASK  {} @ {}", ask.qty, ask.price);
    }
    println!("  ---");
    for bid in &ob.bids {
        println!("  BID  {} @ {}", bid.qty, bid.price);
    }

    // Recent trades
    println!("\n=== Last 5 trades BTC/USDT ===");
    let trades = exchange.trades(&btc, 5).await?;
    for t in &trades {
        println!(
            "  {:?}  {} @ {}  ts={}",
            t.side, t.qty, t.price, t.timestamp_ms
        );
    }

    // Klines
    println!("\n=== Last 3 candles BTC/USDT (1m) ===");
    let candles = exchange.candles(&btc, Interval::M1, 3).await?;
    for c in &candles {
        println!(
            "  O:{} H:{} L:{} C:{} V:{}",
            c.open, c.high, c.low, c.close, c.volume
        );
    }

    Ok(())
}
