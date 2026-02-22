//! Basic REST API example for Bitget.
//!
//! Identical API to Binance/Bybit — demonstrates the unified Exchange trait.
//!
//! Run: cargo run -p gateway-bitget --example basic_rest

use gateway_bitget::Bitget;
use gateway_core::{Exchange, Interval, Symbol};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let bitget = Bitget::public();
    let btc = Symbol::new("BTC", "USDT");

    // ── Exchange Info ──
    let info = bitget.exchange_info().await?;
    println!("=== Bitget Exchange Info ===");
    println!("Trading pairs: {}\n", info.symbols.len());

    // ── Ticker ──
    let tick = bitget.ticker(&btc).await?;
    println!("=== BTC/USDT Ticker ===");
    println!("  Last price : {}", tick.last_price);
    println!("  Bid        : {}", tick.bid.map(|d| d.to_string()).unwrap_or("-".into()));
    println!("  Ask        : {}", tick.ask.map(|d| d.to_string()).unwrap_or("-".into()));
    println!("  Volume 24h : {}", tick.volume_24h);
    println!(
        "  Change 24h : {}%\n",
        tick.price_change_pct_24h
            .map(|d| d.to_string())
            .unwrap_or("-".into())
    );

    // ── Order Book ──
    let ob = bitget.orderbook(&btc, 5).await?;
    println!("=== BTC/USDT Order Book (top 5) ===");
    println!("  {:>14}  {:>14}", "PRICE", "QTY");
    println!("  --- asks ---");
    for lvl in ob.asks.iter().rev() {
        println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
    }
    println!("  --- spread: {} ---", ob.spread().map(|d| d.to_string()).unwrap_or("-".into()));
    for lvl in &ob.bids {
        println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
    }
    println!("  --- bids ---\n");

    // ── Recent Trades ──
    let trades = bitget.trades(&btc, 5).await?;
    println!("=== BTC/USDT Recent Trades ===");
    for t in &trades {
        println!("  {:?}  {} @ {}", t.side, t.qty, t.price);
    }
    println!();

    // ── Candles ──
    let candles = bitget.candles(&btc, Interval::H1, 3).await?;
    println!("=== BTC/USDT Candles (1h, last 3) ===");
    for c in &candles {
        println!(
            "  O={} H={} L={} C={} V={}",
            c.open, c.high, c.low, c.close, c.volume
        );
    }

    Ok(())
}
