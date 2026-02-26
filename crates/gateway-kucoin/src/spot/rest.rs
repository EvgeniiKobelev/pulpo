use crate::spot::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.kucoin.com";
const EXCHANGE: ExchangeId = ExchangeId::Kucoin;

pub struct KucoinRest {
    client: Client,
    base_url: String,
}

impl KucoinRest {
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

    /// Helper: send GET, check HTTP status, deserialize KuCoin response wrapper.
    ///
    /// KuCoin wraps all responses in `{ "code": "200000", "data": ... }`.
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

        let wrapper: KucoinResponse<T> =
            resp.json().await.map_err(|e| GatewayError::Parse {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;

        if wrapper.code != "200000" {
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("API error code: {}", wrapper.code),
                status: None,
            });
        }

        Ok(wrapper.data)
    }

    /// GET /api/v1/symbols
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v1/symbols", self.base_url);
        let data: Vec<KucoinSymbolRaw> = self.fetch(&url).await?;
        Ok(symbols_to_exchange_info(data))
    }

    /// GET /api/v1/market/orderbook/level2_{depth}?symbol=...
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let pair = unified_to_kucoin(symbol);
        // KuCoin supports level2_20 and level2_100
        let level = if depth <= 20 { 20 } else { 100 };
        let url = format!(
            "{}/api/v1/market/orderbook/level2_{}?symbol={}",
            self.base_url, level, pair
        );
        let raw: KucoinOrderBookRaw = self.fetch(&url).await?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /api/v1/market/histories?symbol=...
    ///
    /// Returns last 100 trades (KuCoin does not support a limit parameter).
    pub async fn trades(&self, symbol: &Symbol, _limit: u16) -> Result<Vec<Trade>> {
        let pair = unified_to_kucoin(symbol);
        let url = format!(
            "{}/api/v1/market/histories?symbol={}",
            self.base_url, pair
        );
        let data: Vec<KucoinTradeRaw> = self.fetch(&url).await?;
        let sym = symbol.clone();
        Ok(data
            .into_iter()
            .map(|t| t.into_trade(sym.clone()))
            .collect())
    }

    /// GET /api/v1/market/candles?type={interval}&symbol=...
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let pair = unified_to_kucoin(symbol);
        let kucoin_interval = interval_to_kucoin(interval);
        let url = format!(
            "{}/api/v1/market/candles?type={}&symbol={}",
            self.base_url, kucoin_interval, pair
        );
        let data: Vec<Vec<String>> = self.fetch(&url).await?;
        let sym = symbol.clone();
        let limit = limit as usize;
        let candles: Vec<Candle> = data
            .iter()
            .take(limit.min(1500))
            .filter_map(|row| parse_kline_row(row, sym.clone()))
            .collect();
        Ok(candles)
    }

    /// GET /api/v1/market/stats?symbol=...
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let pair = unified_to_kucoin(symbol);
        let url = format!(
            "{}/api/v1/market/stats?symbol={}",
            self.base_url, pair
        );
        let data: KucoinTickerStatsRaw = self.fetch(&url).await?;
        Ok(data.into_ticker(symbol.clone()))
    }

    /// GET /api/v1/market/allTickers
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v1/market/allTickers", self.base_url);
        let data: KucoinAllTickersResponse = self.fetch(&url).await?;
        let ts = data.time.unwrap_or(0);
        Ok(data
            .ticker
            .into_iter()
            .map(|t| t.into_ticker(ts))
            .collect())
    }

    /// POST /api/v1/bullet-public — get WS connection token and endpoint.
    pub async fn bullet_public(&self) -> Result<KucoinBulletResponse> {
        let url = format!("{}/api/v1/bullet-public", self.base_url);
        let resp = self.client.post(&url).send().await.map_err(|e| {
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
                message: format!("bullet-public HTTP {status}: {body}"),
                status: Some(status),
            });
        }

        let wrapper: KucoinResponse<KucoinBulletResponse> =
            resp.json().await.map_err(|e| GatewayError::Parse {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;

        if wrapper.code != "200000" {
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("bullet-public API error: {}", wrapper.code),
                status: None,
            });
        }

        Ok(wrapper.data)
    }
}
