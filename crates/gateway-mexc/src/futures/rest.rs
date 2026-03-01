use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://contract.mexc.com";
const EXCHANGE: ExchangeId = ExchangeId::MexcFutures;

pub struct MexcFuturesRest {
    client: Client,
    base_url: String,
}

impl MexcFuturesRest {
    pub fn new(config: &ExchangeConfig) -> Self {
        let client = Client::builder()
            .timeout(config.rest.timeout)
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            base_url: BASE_URL.to_string(),
        }
    }

    /// Fetch a MEXC contract API endpoint, unwrapping the
    /// `{"success": true, "code": 0, "data": ...}` envelope.
    async fn fetch<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: EXCHANGE,
                message: e.to_string(),
                status: e.status().map(|s| s.as_u16()),
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("HTTP {status}: {body}"),
                status: Some(status),
            });
        }

        let wrapper: MexcContractResponse<T> =
            resp.json().await.map_err(|e| GatewayError::Parse {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;

        if !wrapper.success {
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("API error code {}", wrapper.code),
                status: None,
            });
        }

        Ok(wrapper.data)
    }

    /// GET /api/v1/contract/detail
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v1/contract/detail", self.base_url);
        let data: Vec<MexcContractDetailRaw> = self.fetch(&url).await?;
        Ok(contracts_to_exchange_info(data))
    }

    /// GET /api/v1/contract/depth/{symbol}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let pair = unified_to_mexc_futures(symbol);
        let depth = depth.min(100);
        let url = format!(
            "{}/api/v1/contract/depth/{}?limit={}",
            self.base_url, pair, depth
        );
        let raw: MexcFuturesDepthRaw = self.fetch(&url).await?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /api/v1/contract/deals/{symbol}
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let pair = unified_to_mexc_futures(symbol);
        let limit = limit.min(100);
        let url = format!(
            "{}/api/v1/contract/deals/{}?limit={}",
            self.base_url, pair, limit
        );
        let data: Vec<MexcFuturesDealRaw> = self.fetch(&url).await?;
        let sym = symbol.clone();
        Ok(data.into_iter().map(|d| d.into_trade(sym.clone())).collect())
    }

    /// GET /api/v1/contract/kline/{symbol}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let pair = unified_to_mexc_futures(symbol);
        let limit = limit.min(2000);
        let iv = interval_to_mexc_futures(interval);
        let url = format!(
            "{}/api/v1/contract/kline/{}?interval={}&limit={}",
            self.base_url, pair, iv, limit
        );

        // MEXC futures klines: the data field may be a map with time/open/close/high/low/vol
        // or a top-level wrapper around individual kline objects.
        let data: Vec<MexcFuturesKlineRaw> = self.fetch(&url).await?;
        let sym = symbol.clone();
        Ok(data
            .into_iter()
            .filter_map(|k| k.into_candle(sym.clone()))
            .collect())
    }

    /// GET /api/v1/contract/ticker?symbol=X
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let pair = unified_to_mexc_futures(symbol);
        let url = format!(
            "{}/api/v1/contract/ticker?symbol={}",
            self.base_url, pair
        );
        let raw: MexcFuturesTickerRaw = self.fetch(&url).await?;
        Ok(raw.into_ticker())
    }

    /// GET /api/v1/contract/ticker (all)
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v1/contract/ticker", self.base_url);
        let data: Vec<MexcFuturesTickerRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|t| t.into_ticker()).collect())
    }

    /// GET /api/v1/contract/ticker?symbol=X — returns raw ticker for futures-specific data.
    pub async fn ticker_raw(&self, symbol: &Symbol) -> Result<MexcFuturesTickerRaw> {
        let pair = unified_to_mexc_futures(symbol);
        let url = format!(
            "{}/api/v1/contract/ticker?symbol={}",
            self.base_url, pair
        );
        self.fetch(&url).await
    }

    /// GET /api/v1/contract/funding_rate/{symbol}
    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let pair = unified_to_mexc_futures(symbol);
        let url = format!(
            "{}/api/v1/contract/funding_rate/{}",
            self.base_url, pair
        );
        let raw: MexcFuturesFundingRateRaw = self.fetch(&url).await?;
        Ok(raw.into_funding_rate())
    }

    /// GET /api/v1/contract/fair_price/{symbol}
    pub async fn fair_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let pair = unified_to_mexc_futures(symbol);
        let url = format!(
            "{}/api/v1/contract/fair_price/{}",
            self.base_url, pair
        );
        let raw: MexcFuturesFairPriceRaw = self.fetch(&url).await?;
        Ok(raw.into_mark_price())
    }

    /// GET /api/v1/contract/open_interest/{symbol}
    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let pair = unified_to_mexc_futures(symbol);
        let url = format!(
            "{}/api/v1/contract/open_interest/{}",
            self.base_url, pair
        );
        let raw: MexcFuturesOpenInterestRaw = self.fetch(&url).await?;
        Ok(raw.into_open_interest())
    }
}
