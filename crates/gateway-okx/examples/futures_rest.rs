//! Futures REST API example for OKX.
//!
//! Demonstrates futures-specific market data: funding rate, mark price,
//! open interest, and orderbook on perpetual SWAP contracts.
//!
//! Run: cargo run -p gateway-okx --example futures_rest

use gateway_core::{Exchange, FuturesExchange, Symbol};
use gateway_okx::OkxFutures;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let futures = OkxFutures::public();
    let btc = Symbol::new("BTC", "USDT");

    // -- Exchange Info --
    let info = futures.exchange_info().await?;
    println!("=== OKX Futures Exchange Info ===");
    println!("Trading pairs: {}\n", info.symbols.len());

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
    println!();

    // -- Funding Rate --
    let fr = futures.funding_rate(&btc).await?;
    println!("=== BTC/USDT Funding Rate ===");
    println!("  Rate: {}", fr.rate);
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
    println!("  Open interest       : {} BTC", oi.open_interest);
    println!("  Open interest value : {} USD", oi.open_interest_value);
    println!();

    // -- Order Book (top 5) --
    let ob = futures.orderbook(&btc, 5).await?;
    println!("=== BTC/USDT Futures Order Book (top 5) ===");
    println!("  {:>14}  {:>14}", "PRICE", "QTY");
    println!("  --- asks ---");
    for lvl in ob.asks.iter().rev() {
        println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
    }
    println!(
        "  --- spread: {} ---",
        ob.spread().map(|d| d.to_string()).unwrap_or("-".into())
    );
    for lvl in &ob.bids {
        println!("  {:>14}  {:>14}", lvl.price, lvl.qty);
    }
    println!("  --- bids ---");

    Ok(())
}
