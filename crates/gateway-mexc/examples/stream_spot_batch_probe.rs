//! Живой пробник: подписка на все USDT-пары spot одним batch-вызовом,
//! счёт уникальных символов с данными за 30 с.
use futures::StreamExt;
use gateway_core::traits::Exchange;
use gateway_core::types::SymbolStatus;
use gateway_mexc::MexcSpot;
use std::collections::HashSet;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    let ex = MexcSpot::public();
    let info = ex.exchange_info().await?;
    let symbols: Vec<_> = info
        .symbols
        .into_iter()
        .filter(|s| s.status == SymbolStatus::Trading && s.symbol.quote == "USDT")
        .map(|s| s.symbol)
        .collect();
    println!("subscribing {} symbols", symbols.len());
    let mut stream = ex.stream_trades_batch(&symbols).await?;
    let mut seen = HashSet::new();
    let mut trades = 0u64;
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            t = stream.next() => match t {
                Some(t) => { trades += 1; seen.insert(t.symbol.to_string()); }
                None => break,
            }
        }
    }
    println!("30s: trades={trades} distinct_symbols={}", seen.len());
    Ok(())
}
