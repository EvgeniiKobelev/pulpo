pub mod spot;
pub mod futures;

pub use spot::BitgetSpot;
pub use futures::BitgetFutures;

/// Backwards-compatible alias.
pub type Bitget = BitgetSpot;
