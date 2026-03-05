//! Futures REST API example for XT.
//!
//! Demonstrates futures-specific market data: funding rate, mark price,
//! open interest, and orderbook on perpetual contracts.
//!
//! Run: cargo run -p gateway-xt --example futures_rest

use gateway_core::{Exchange, FuturesExchange, Interval, Symbol};
use gateway_xt::XtFutures;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = XtFutures::public();
    let btc = Symbol::new("BTC", "USDT");

    // -- Exchange Info --
    let info = futures.exchange_info().await?;
    println!("=== XT Futures Exchange Info ===");
    println!("Trading pairs: {}\n", info.symbols.len());
    for s in info.symbols.iter().take(10) {
        println!(
            "  {} (raw: {}) precision: base={} quote={} tick={}",
            s.symbol,
            s.raw_symbol,
            s.base_precision,
            s.quote_precision,
            s.tick_size
                .map(|d| d.to_string())
                .unwrap_or("-".into())
        );
    }
    println!();

    // -- Ticker --
    let ticker = futures.ticker(&btc).await?;
    println!("=== BTC/USDT Futures Ticker ===");
    println!("  Last price : {}", ticker.last_price);
    println!(
        "  Bid        : {}",
        ticker.bid.map(|d| d.to_string()).unwrap_or("-".into())
    );
    println!(
        "  Ask        : {}",
        ticker.ask.map(|d| d.to_string()).unwrap_or("-".into())
    );
    println!("  Volume 24h : {}", ticker.volume_24h);
    println!(
        "  Change 24h : {}%",
        ticker
            .price_change_pct_24h
            .map(|d| d.to_string())
            .unwrap_or("-".into())
    );
    println!();

    // -- Funding Rate --
    let fr = futures.funding_rate(&btc).await?;
    println!("=== BTC/USDT Funding Rate ===");
    println!("  Rate              : {}", fr.rate);
    println!("  Next funding time : {}", fr.next_funding_time_ms);
    println!();

    // -- Mark Price --
    let mp = futures.mark_price(&btc).await?;
    println!("=== BTC/USDT Mark Price ===");
    println!("  Mark price  : {}", mp.mark_price);
    println!("  Index price : {}", mp.index_price);
    println!();

    // -- Open Interest --
    let oi = futures.open_interest(&btc).await?;
    println!("=== BTC/USDT Open Interest ===");
    println!("  OI        : {}", oi.open_interest);
    println!("  OI value  : {}", oi.open_interest_value);
    println!();

    // -- Order Book (top 5) --
    let ob = futures.orderbook(&btc, 5).await?;
    println!("=== BTC/USDT Futures Order Book (top 5) ===");
    println!("  {:>14}  {:>14}", "PRICE", "QTY");
    println!("  --- asks ---");
    for lvl in ob.asks.iter().take(5).rev() {
        println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
    }
    println!(
        "  --- spread: {} ---",
        ob.spread().map(|d| d.to_string()).unwrap_or("-".into())
    );
    for lvl in ob.bids.iter().take(5) {
        println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
    }
    println!("  --- bids ---");
    println!();

    // -- Recent Trades (last 5) --
    let trades = futures.trades(&btc, 5).await?;
    println!("=== BTC/USDT Recent Trades (last {}) ===", trades.len());
    for t in &trades {
        println!(
            "  {:?} {} @ {} ({})",
            t.side, t.qty, t.price, t.timestamp_ms
        );
    }
    println!();

    // -- Candles (1h, last 5) --
    let candles = futures.candles(&btc, Interval::H1, 5).await?;
    println!("=== BTC/USDT 1h Candles (last {}) ===", candles.len());
    for c in &candles {
        println!(
            "  O={} H={} L={} C={} V={}",
            c.open, c.high, c.low, c.close, c.volume
        );
    }

    Ok(())
}
