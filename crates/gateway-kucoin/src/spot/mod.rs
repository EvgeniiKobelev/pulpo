pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct KucoinSpot {
    config: ExchangeConfig,
    rest: rest::KucoinRest,
}

impl KucoinSpot {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::KucoinRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for KucoinSpot {
    fn id(&self) -> ExchangeId {
        ExchangeId::Kucoin
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

    async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        self.rest.candles(symbol, interval, limit).await
    }

    async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        self.rest.ticker(symbol).await
    }

    async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        self.rest.all_tickers().await
    }

    async fn stream_orderbook(&self, symbol: &Symbol) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbook(&self.config, symbol).await
    }

    async fn stream_trades(&self, symbol: &Symbol) -> Result<BoxStream<Trade>> {
        ws::stream_trades(&self.config, symbol).await
    }

    async fn stream_candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
    ) -> Result<BoxStream<Candle>> {
        ws::stream_candles(&self.config, symbol, interval).await
    }

    async fn stream_orderbooks_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbooks_batch(&self.config, symbols).await
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        ws::stream_trades_batch(&self.config, symbols).await
    }
}
