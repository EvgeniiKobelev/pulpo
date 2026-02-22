pub mod spot;
pub mod futures;

pub use spot::BybitSpot;
pub use futures::BybitFutures;

/// Backwards-compatible alias.
pub type Bybit = BybitSpot;
