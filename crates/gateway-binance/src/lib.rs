pub mod spot;
pub mod futures;
pub(crate) mod local_book;
pub(crate) mod rate_limit;

pub use spot::BinanceSpot;
pub use futures::BinanceFutures;

/// Backwards-compatible alias.
pub type Binance = BinanceSpot;
