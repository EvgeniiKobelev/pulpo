pub mod spot;

pub use spot::BinanceSpot;

/// Backwards-compatible alias.
pub type Binance = BinanceSpot;
