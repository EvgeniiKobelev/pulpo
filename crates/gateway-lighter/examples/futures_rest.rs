//! Futures REST API example for Lighter DEX.
//!
//! Demonstrates futures-specific market data: exchange info, ticker, funding
//! rate, mark price, open interest, recent trades, candles and order book on
//! Lighter perpetual contracts.
//!
//! All Lighter perp markets are quoted in USDC.
//!
//! Run: cargo run -p gateway-lighter --example futures_rest

use gateway_core::{Exchange, FuturesExchange, Interval, Symbol};
use gateway_lighter::LighterFutures;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let lighter = LighterFutures::public();
    let eth = Symbol::new("ETH", "USDC");

    // -- Exchange Info --
    let info = lighter.exchange_info().await?;
    println!("=== Lighter Futures Exchange Info ===");
    println!("Perp markets: {}", info.symbols.len());
    for si in &info.symbols {
        println!(
            "  {} (raw: {})  status={:?}  base_prec={}  price_prec={}",
            si.symbol, si.raw_symbol, si.status, si.base_precision, si.quote_precision,
        );
    }
    println!();

    // -- All Tickers --
    let tickers = lighter.all_tickers().await?;
    println!("=== All Tickers ===");
    for t in &tickers {
        println!(
            "  {}  last={}  chg={}%",
            t.symbol,
            t.last_price,
            t.price_change_pct_24h
                .map(|d| d.to_string())
                .unwrap_or("-".into()),
        );
    }
    println!();

    // -- Single Ticker --
    let ticker = lighter.ticker(&eth).await?;
    println!("=== ETH/USDC Futures Ticker ===");
    println!("  Last price : {}", ticker.last_price);
    println!(
        "  Change 24h : {}%",
        ticker
            .price_change_pct_24h
            .map(|d| d.to_string())
            .unwrap_or("-".into()),
    );
    println!();

    // -- Funding Rate --
    let fr = lighter.funding_rate(&eth).await?;
    println!("=== ETH/USDC Funding Rate ===");
    println!("  Rate: {}", fr.rate);
    println!();

    // -- Mark Price --
    let mp = lighter.mark_price(&eth).await?;
    println!("=== ETH/USDC Mark Price ===");
    println!("  Mark price  : {}", mp.mark_price);
    println!("  Index price : {}", mp.index_price);
    println!();

    // -- Open Interest --
    let oi = lighter.open_interest(&eth).await?;
    println!("=== ETH/USDC Open Interest ===");
    println!("  Open interest: {} ETH", oi.open_interest);
    println!();

    // -- Order Book (top 5) --
    let ob = lighter.orderbook(&eth, 5).await?;
    println!("=== ETH/USDC Futures Order Book ===");
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

    // -- Recent Trades --
    let trades = lighter.trades(&eth, 5).await?;
    println!("=== ETH/USDC Recent Trades ===");
    for t in &trades {
        println!("  {:?}  {} @ {}", t.side, t.qty, t.price);
    }
    println!();

    // -- Candles --
    let candles = lighter.candles(&eth, Interval::H1, 3).await?;
    println!("=== ETH/USDC Candles (1h, last 3) ===");
    for c in &candles {
        println!(
            "  O={} H={} L={} C={} V={}",
            c.open, c.high, c.low, c.close, c.volume
        );
    }

    Ok(())
}
