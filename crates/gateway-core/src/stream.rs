use crate::types::*;
use std::pin::Pin;
use futures::Stream;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    OrderBook(OrderBook),
    Trade(Trade),
    Candle(Candle),
    Ticker(Ticker),
    FundingRate(FundingRate),
    MarkPrice(MarkPrice),
    Liquidation(Liquidation),
    Info(String),
}

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

pub struct Subscription {
    _cancel: tokio::sync::oneshot::Sender<()>,
}

impl Subscription {
    pub fn new(cancel: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { _cancel: cancel }
    }
}
