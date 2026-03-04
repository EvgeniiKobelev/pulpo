pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct BitunixFutures {
    config: ExchangeConfig,
    rest: rest::BitunixFuturesRest,
}

impl BitunixFutures {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BitunixFuturesRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for BitunixFutures {
    fn id(&self) -> ExchangeId {
        ExchangeId::BitunixFutures
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

    async fn trades(&self, _symbol: &Symbol, _limit: u16) -> Result<Vec<Trade>> {
        // Bitunix does not expose public recent trades via REST.
        Ok(vec![])
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
impl FuturesExchange for BitunixFutures {
    async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        self.rest.funding_rate(symbol).await
    }

    async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        self.rest.mark_price(symbol).await
    }

    async fn open_interest(&self, _symbol: &Symbol) -> Result<OpenInterest> {
        // Bitunix does not expose public open interest via REST.
        Ok(OpenInterest {
            exchange: ExchangeId::BitunixFutures,
            symbol: _symbol.clone(),
            open_interest: rust_decimal::Decimal::ZERO,
            open_interest_value: rust_decimal::Decimal::ZERO,
            timestamp_ms: now_ms(),
        })
    }

    async fn liquidations(&self, _symbol: &Symbol, _limit: u16) -> Result<Vec<Liquidation>> {
        // Bitunix does not expose public liquidation data via REST.
        Ok(vec![])
    }

    async fn stream_mark_price(&self, symbol: &Symbol) -> Result<BoxStream<MarkPrice>> {
        ws::stream_mark_price(&self.config, symbol).await
    }

    async fn stream_liquidations(&self, _symbol: &Symbol) -> Result<BoxStream<Liquidation>> {
        // Bitunix does not expose public liquidation stream.
        Ok(Box::pin(futures::stream::empty()))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
