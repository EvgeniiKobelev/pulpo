pub mod spot;
pub mod futures;

pub use spot::OkxSpot;
pub use futures::OkxFutures;

pub type Okx = OkxSpot;
