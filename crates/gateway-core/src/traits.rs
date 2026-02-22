use async_trait::async_trait;
use crate::{
    config::ExchangeConfig,
    error::Result,
    stream::BoxStream,
    types::*,
};

#[async_trait]
pub trait Exchange: Send + Sync + 'static {
    fn id(&self) -> ExchangeId;
    fn config(&self) -> &ExchangeConfig;

    async fn exchange_info(&self) -> Result<ExchangeInfo>;
    async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook>;
    async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>>;
    async fn candles(&self, symbol: &Symbol, interval: Interval, limit: u16) -> Result<Vec<Candle>>;
    async fn ticker(&self, symbol: &Symbol) -> Result<Ticker>;
    async fn all_tickers(&self) -> Result<Vec<Ticker>>;

    async fn stream_orderbook(&self, symbol: &Symbol) -> Result<BoxStream<OrderBook>>;
    async fn stream_trades(&self, symbol: &Symbol) -> Result<BoxStream<Trade>>;
    async fn stream_candles(&self, symbol: &Symbol, interval: Interval) -> Result<BoxStream<Candle>>;

    async fn stream_orderbooks_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<OrderBook>> {
        use futures::stream::SelectAll;
        let mut all = SelectAll::new();
        for sym in symbols {
            all.push(self.stream_orderbook(sym).await?);
        }
        Ok(Box::pin(all))
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        use futures::stream::SelectAll;
        let mut all = SelectAll::new();
        for sym in symbols {
            all.push(self.stream_trades(sym).await?);
        }
        Ok(Box::pin(all))
    }
}

#[async_trait]
pub trait ExchangeTrading: Exchange {
    async fn balances(&self) -> Result<Vec<Balance>>;
    async fn place_order(&self, order: &NewOrder) -> Result<OrderResponse>;
    async fn cancel_order(&self, symbol: &Symbol, order_id: &str) -> Result<()>;
    async fn open_orders(&self, symbol: Option<&Symbol>) -> Result<Vec<Order>>;
}

#[derive(Debug, Clone)]
pub struct Balance {
    pub asset: String,
    pub free: rust_decimal::Decimal,
    pub locked: rust_decimal::Decimal,
}

#[derive(Debug, Clone)]
pub struct NewOrder {
    pub symbol: Symbol,
    pub side: Side,
    pub order_type: OrderType,
    pub qty: rust_decimal::Decimal,
    pub price: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, Copy)]
pub enum OrderType { Market, Limit }

#[derive(Debug, Clone)]
pub struct OrderResponse {
    pub order_id: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub order_type: OrderType,
    pub price: rust_decimal::Decimal,
    pub qty: rust_decimal::Decimal,
    pub filled_qty: rust_decimal::Decimal,
}

#[async_trait]
pub trait FuturesExchange: Exchange {
    async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate>;
    async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice>;
    async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest>;
    async fn liquidations(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Liquidation>>;

    async fn stream_mark_price(&self, symbol: &Symbol) -> Result<BoxStream<MarkPrice>>;
    async fn stream_liquidations(&self, symbol: &Symbol) -> Result<BoxStream<Liquidation>>;
}
