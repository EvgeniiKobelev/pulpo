pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct BinanceFutures {
    config: ExchangeConfig,
    rest: rest::BinanceFuturesRest,
}

impl BinanceFutures {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BinanceFuturesRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for BinanceFutures {
    fn id(&self) -> ExchangeId {
        ExchangeId::BinanceFutures
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

    async fn stream_orderbook(&self, symbol: &Symbol) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbook(&self.config, symbol).await
    }

    async fn stream_trades(&self, symbol: &Symbol) -> Result<BoxStream<Trade>> {
        ws::stream_trades(&self.config, symbol).await
    }

    async fn stream_candles(&self, symbol: &Symbol, interval: Interval) -> Result<BoxStream<Candle>> {
        ws::stream_candles(&self.config, symbol, interval).await
    }

    async fn stream_orderbooks_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbooks_combined(&self.config, symbols).await
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        ws::stream_trades_combined(&self.config, symbols).await
    }
}

#[async_trait]
impl FuturesExchange for BinanceFutures {
    async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let raw = self.rest.premium_index(symbol).await?;
        Ok(raw.into_funding_rate())
    }

    async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let raw = self.rest.premium_index(symbol).await?;
        Ok(raw.into_mark_price())
    }

    async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let raw = self.rest.open_interest(symbol).await?;
        Ok(raw.into_open_interest())
    }

    async fn liquidations(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Liquidation>> {
        let raw = self.rest.force_orders(symbol, limit).await?;
        Ok(raw.into_iter().map(|r| r.into_liquidation()).collect())
    }

    async fn stream_mark_price(&self, symbol: &Symbol) -> Result<BoxStream<MarkPrice>> {
        ws::stream_mark_price(&self.config, symbol).await
    }

    async fn stream_liquidations(&self, symbol: &Symbol) -> Result<BoxStream<Liquidation>> {
        ws::stream_liquidations(&self.config, symbol).await
    }
}
