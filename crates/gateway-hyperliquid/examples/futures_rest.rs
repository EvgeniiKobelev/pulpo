//! Futures REST API example for Hyperliquid.
//!
//! Demonstrates futures-specific market data: funding rate, mark price,
//! open interest, and orderbook on perpetual contracts.
//!
//! Run: cargo run -p gateway-hyperliquid --example futures_rest

use gateway_hyperliquid::HyperliquidFutures;
use gateway_core::{Exchange, FuturesExchange, Symbol};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = HyperliquidFutures::public();
    let btc = Symbol::new("BTC", "USDC");

    // -- Exchange Info --
    let info = futures.exchange_info().await?;
    println!("=== Hyperliquid Futures Exchange Info ===");
    println!("Trading pairs: {}\n", info.symbols.len());

    // -- Ticker --
    let ticker = futures.ticker(&btc).await?;
    println!("=== BTC/USDC Futures Ticker ===");
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
    println!();

    // -- Funding Rate --
    let fr = futures.funding_rate(&btc).await?;
    println!("=== BTC/USDC Funding Rate ===");
    println!("  Rate: {}", fr.rate);
    println!();

    // -- Mark Price --
    let mp = futures.mark_price(&btc).await?;
    println!("=== BTC/USDC Mark Price ===");
    println!("  Mark price  : {}", mp.mark_price);
    println!("  Index price : {}", mp.index_price);
    println!();

    // -- Open Interest --
    let oi = futures.open_interest(&btc).await?;
    println!("=== BTC/USDC Open Interest ===");
    println!("  Open interest       : {} BTC", oi.open_interest);
    println!("  Open interest value : {} USDC", oi.open_interest_value);
    println!();

    // -- Order Book (top 5) --
    let ob = futures.orderbook(&btc, 5).await?;
    println!("=== BTC/USDC Futures Order Book (top 5) ===");
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

    Ok(())
}
