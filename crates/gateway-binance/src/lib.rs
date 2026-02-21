pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct Binance {
    config: ExchangeConfig,
    rest: rest::BinanceRest,
}

impl Binance {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BinanceRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for Binance {
    fn id(&self) -> ExchangeId {
        ExchangeId::Binance
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

    // WS stubs -- will be implemented in Task 6
    async fn stream_orderbook(&self, _symbol: &Symbol) -> Result<BoxStream<OrderBook>> {
        todo!("WS not yet implemented")
    }

    async fn stream_trades(&self, _symbol: &Symbol) -> Result<BoxStream<Trade>> {
        todo!("WS not yet implemented")
    }

    async fn stream_candles(&self, _symbol: &Symbol, _interval: Interval) -> Result<BoxStream<Candle>> {
        todo!("WS not yet implemented")
    }
}
