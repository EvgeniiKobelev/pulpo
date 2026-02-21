pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct Bybit {
    config: ExchangeConfig,
    rest: rest::BybitRest,
}

impl Bybit {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BybitRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for Bybit {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bybit
    }

    fn config(&self) -> &ExchangeConfig {
        &self.config
    }

    async fn exchange_info(&self) -> Result<ExchangeInfo> {
        self.rest.exchange_info().await
    }

    async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        self.rest.orderbook(symbol, depth).await
    }

    async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        self.rest.trades(symbol, limit).await
    }

    async fn candles(&self, symbol: &Symbol, interval: Interval, limit: u16) -> Result<Vec<Candle>> {
        self.rest.candles(symbol, interval, limit).await
    }

    async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        self.rest.ticker(symbol).await
    }

    async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        self.rest.all_tickers().await
    }

    // WS stubs — to be implemented in Task 9
    async fn stream_orderbook(&self, _symbol: &Symbol) -> Result<BoxStream<OrderBook>> {
        todo!()
    }

    async fn stream_trades(&self, _symbol: &Symbol) -> Result<BoxStream<Trade>> {
        todo!()
    }

    async fn stream_candles(&self, _symbol: &Symbol, _interval: Interval) -> Result<BoxStream<Candle>> {
        todo!()
    }
}
