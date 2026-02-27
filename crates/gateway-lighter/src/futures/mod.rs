pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;
use rust_decimal::Decimal;
use std::str::FromStr;

pub struct LighterFutures {
    config: ExchangeConfig,
    rest: rest::LighterRest,
}

impl LighterFutures {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::LighterRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }

    /// Resolve a unified Symbol to Lighter market_id.
    async fn market_id(&self, symbol: &Symbol) -> Result<u16> {
        self.rest.market_id(symbol).await
    }

    /// Build a list of (market_id, Symbol) pairs from symbols.
    async fn resolve_pairs(&self, symbols: &[Symbol]) -> Result<Vec<(u16, Symbol)>> {
        let cache = self.rest.market_cache().await?;
        symbols
            .iter()
            .map(|s| {
                cache
                    .market_id(s)
                    .map(|mid| (mid, s.clone()))
                    .ok_or_else(|| GatewayError::SymbolNotFound {
                        exchange: ExchangeId::LighterFutures,
                        symbol: s.to_string(),
                    })
            })
            .collect()
    }
}

#[async_trait]
impl Exchange for LighterFutures {
    fn id(&self) -> ExchangeId {
        ExchangeId::LighterFutures
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
        let mid = self.market_id(symbol).await?;
        ws::stream_orderbook(&self.config, mid, symbol.clone()).await
    }

    async fn stream_trades(&self, symbol: &Symbol) -> Result<BoxStream<Trade>> {
        let mid = self.market_id(symbol).await?;
        ws::stream_trades(&self.config, mid, symbol.clone()).await
    }

    async fn stream_candles(
        &self,
        _symbol: &Symbol,
        _interval: Interval,
    ) -> Result<BoxStream<Candle>> {
        Err(GatewayError::Other(
            "Lighter does not support WebSocket candlestick streaming".into(),
        ))
    }

    async fn stream_orderbooks_batch(
        &self,
        symbols: &[Symbol],
    ) -> Result<BoxStream<OrderBook>> {
        let pairs = self.resolve_pairs(symbols).await?;
        ws::stream_orderbooks_batch(&self.config, &pairs).await
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        let pairs = self.resolve_pairs(symbols).await?;
        ws::stream_trades_batch(&self.config, &pairs).await
    }
}

#[async_trait]
impl FuturesExchange for LighterFutures {
    async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        self.rest.funding_rate(symbol).await
    }

    async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let detail = self.rest.market_details(symbol).await?;
        Ok(MarkPrice {
            exchange: ExchangeId::LighterFutures,
            symbol: symbol.clone(),
            mark_price: Decimal::from_str(&detail.last_trade_price.to_string())
                .unwrap_or_default(),
            index_price: Decimal::ZERO,
            timestamp_ms: 0,
        })
    }

    async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let detail = self.rest.market_details(symbol).await?;
        let oi = Decimal::from_str(&detail.open_interest.to_string()).unwrap_or_default();

        Ok(OpenInterest {
            exchange: ExchangeId::LighterFutures,
            symbol: symbol.clone(),
            open_interest: oi,
            open_interest_value: Decimal::ZERO,
            timestamp_ms: 0,
        })
    }

    async fn liquidations(&self, _symbol: &Symbol, _limit: u16) -> Result<Vec<Liquidation>> {
        // Lighter liquidation data requires authentication.
        Ok(vec![])
    }

    async fn stream_mark_price(&self, symbol: &Symbol) -> Result<BoxStream<MarkPrice>> {
        let mid = self.market_id(symbol).await?;
        ws::stream_mark_price(&self.config, mid, symbol.clone()).await
    }

    async fn stream_liquidations(
        &self,
        _symbol: &Symbol,
    ) -> Result<BoxStream<Liquidation>> {
        Err(GatewayError::Other(
            "Lighter does not support public liquidation streaming".into(),
        ))
    }
}
