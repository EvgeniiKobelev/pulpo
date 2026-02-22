pub mod spot;
pub mod futures;

pub use spot::BinanceSpot;
pub use futures::BinanceFutures;

/// Backwards-compatible alias.
pub type Binance = BinanceSpot;
