pub mod spot;

pub use spot::BybitSpot;

/// Backwards-compatible alias.
pub type Bybit = BybitSpot;
