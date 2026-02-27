use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::OnceCell;

const BASE_URL: &str = "https://mainnet.zklighter.elliot.ai";

pub struct LighterRest {
    client: Client,
    base_url: String,
    markets: Arc<OnceCell<MarketCache>>,
}

impl LighterRest {
    pub fn new(config: &ExchangeConfig) -> Self {
        let client = Client::builder()
            .timeout(config.rest.timeout)
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            base_url: BASE_URL.to_string(),
            markets: Arc::new(OnceCell::new()),
        }
    }

    /// Ensure the market cache is populated. Returns a reference to it.
    pub async fn ensure_markets(&self) -> Result<&MarketCache> {
        self.markets
            .get_or_try_init(|| async {
                let resp = self.fetch_order_books_raw().await?;
                Ok(resp.into_market_cache())
            })
            .await
    }

    /// Look up the market_id for a given symbol.
    pub async fn market_id(&self, symbol: &Symbol) -> Result<u16> {
        let cache = self.ensure_markets().await?;
        cache.market_id(symbol).ok_or_else(|| GatewayError::SymbolNotFound {
            exchange: ExchangeId::LighterFutures,
            symbol: symbol.to_string(),
        })
    }

    /// Returns a clone of the full market cache.
    pub async fn market_cache(&self) -> Result<MarketCache> {
        Ok(self.ensure_markets().await?.clone())
    }

    // -----------------------------------------------------------------------
    // Raw fetch helpers
    // -----------------------------------------------------------------------

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::LighterFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::LighterFutures,
                message: body,
                status: Some(status),
            });
        }

        resp.json::<T>().await.map_err(|e| GatewayError::Parse {
            exchange: ExchangeId::LighterFutures,
            message: e.to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // REST endpoints
    // -----------------------------------------------------------------------

    /// GET /api/v1/orderBooks?filter=perp
    async fn fetch_order_books_raw(&self) -> Result<LighterOrderBooksResponse> {
        let url = format!("{}/api/v1/orderBooks?filter=perp", self.base_url);
        self.get(&url).await
    }

    /// GET /api/v1/orderBooks?filter=perp → ExchangeInfo
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let resp = self.fetch_order_books_raw().await?;
        Ok(resp.into_exchange_info())
    }

    /// GET /api/v1/orderBookOrders?market_id=X&limit=Y
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let mid = self.market_id(symbol).await?;
        let limit = if depth == 0 { 50 } else { depth.min(250) };
        let url = format!(
            "{}/api/v1/orderBookOrders?market_id={}&limit={}",
            self.base_url, mid, limit
        );
        let resp: LighterOrderBookOrdersResponse = self.get(&url).await?;
        Ok(resp.into_orderbook(symbol.clone()))
    }

    /// GET /api/v1/recentTrades?market_id=X&limit=Y
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let mid = self.market_id(symbol).await?;
        let limit = limit.min(100);
        let url = format!(
            "{}/api/v1/recentTrades?market_id={}&limit={}",
            self.base_url, mid, limit
        );
        let resp: LighterRecentTradesResponse = self.get(&url).await?;
        Ok(resp
            .trades
            .into_iter()
            .map(|t| t.into_trade(symbol.clone()))
            .collect())
    }

    /// GET /api/v1/candles?market_id=X&resolution=Y&count_back=Z&start_timestamp=S&end_timestamp=E
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let mid = self.market_id(symbol).await?;
        let resolution = interval_to_lighter(interval);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let interval_ms = interval.as_secs() * 1000;
        let start_ms = now_ms.saturating_sub(interval_ms * limit as u64);
        let url = format!(
            "{}/api/v1/candles?market_id={}&resolution={}&start_timestamp={}&end_timestamp={}&count_back={}",
            self.base_url, mid, resolution, start_ms, now_ms, limit
        );
        let resp: LighterCandlesResponse = self.get(&url).await?;
        Ok(resp
            .c
            .into_iter()
            .map(|c| c.into_candle(symbol.clone(), interval))
            .collect())
    }

    /// GET /api/v1/orderBookDetails?market_id=X  → single ticker
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let mid = self.market_id(symbol).await?;
        let url = format!(
            "{}/api/v1/orderBookDetails?market_id={}",
            self.base_url, mid
        );
        let resp: LighterOrderBookDetailsResponse = self.get(&url).await?;
        resp.order_book_details
            .into_iter()
            .next()
            .map(|d| d.into_ticker())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::LighterFutures,
                symbol: symbol.to_string(),
            })
    }

    /// GET /api/v1/orderBookDetails?filter=perp  → all tickers
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!(
            "{}/api/v1/orderBookDetails?filter=perp",
            self.base_url
        );
        let resp: LighterOrderBookDetailsResponse = self.get(&url).await?;
        Ok(resp
            .order_book_details
            .into_iter()
            .map(|d| d.into_ticker())
            .collect())
    }

    /// GET /api/v1/funding-rates → funding rate for a specific symbol
    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let url = format!("{}/api/v1/funding-rates", self.base_url);
        let resp: LighterFundingRatesResponse = self.get(&url).await?;

        let cache = self.ensure_markets().await?;

        // Find the rate for the requested symbol from the "lighter" exchange.
        resp.funding_rates
            .into_iter()
            .filter(|r| r.exchange == "lighter")
            .find_map(|r| {
                let mid = r.market_id;
                let sym = cache.symbol(mid)?;
                if &sym == symbol {
                    Some(FundingRate {
                        exchange: ExchangeId::LighterFutures,
                        symbol: sym,
                        rate: rust_decimal::Decimal::from_str_exact(&r.rate.to_string())
                            .unwrap_or_default(),
                        next_funding_time_ms: 0,
                        timestamp_ms: 0,
                    })
                } else {
                    None
                }
            })
            .ok_or_else(|| GatewayError::Rest {
                exchange: ExchangeId::LighterFutures,
                message: format!("no funding rate for {}", symbol),
                status: None,
            })
    }

    /// GET /api/v1/orderBookDetails → mark price + open interest
    pub async fn market_details(&self, symbol: &Symbol) -> Result<LighterOrderBookDetail> {
        let mid = self.market_id(symbol).await?;
        let url = format!(
            "{}/api/v1/orderBookDetails?market_id={}",
            self.base_url, mid
        );
        let resp: LighterOrderBookDetailsResponse = self.get(&url).await?;
        resp.order_book_details
            .into_iter()
            .next()
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::LighterFutures,
                symbol: symbol.to_string(),
            })
    }
}
