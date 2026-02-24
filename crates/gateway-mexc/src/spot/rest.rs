use crate::spot::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.mexc.com";
const EXCHANGE: ExchangeId = ExchangeId::Mexc;

pub struct MexcRest {
    client: Client,
    base_url: String,
}

impl MexcRest {
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

    /// Helper: send GET, check HTTP status, deserialize JSON.
    ///
    /// MEXC REST responses have no wrapper — data is returned directly.
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

        resp.json::<T>().await.map_err(|e| GatewayError::Parse {
            exchange: EXCHANGE,
            message: e.to_string(),
        })
    }

    /// GET /api/v3/exchangeInfo
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v3/exchangeInfo", self.base_url);
        let data: MexcExchangeInfoResponse = self.fetch(&url).await?;
        Ok(symbols_to_exchange_info(data))
    }

    /// GET /api/v3/depth?symbol={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let pair = unified_to_mexc(symbol);
        let depth = depth.min(5000);
        let url = format!(
            "{}/api/v3/depth?symbol={}&limit={}",
            self.base_url, pair, depth
        );
        let raw: MexcOrderBookRaw = self.fetch(&url).await?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /api/v3/trades?symbol={}&limit={}
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let pair = unified_to_mexc(symbol);
        let limit = limit.min(1000);
        let url = format!(
            "{}/api/v3/trades?symbol={}&limit={}",
            self.base_url, pair, limit
        );
        let data: Vec<MexcTradeRaw> = self.fetch(&url).await?;
        let sym = symbol.clone();
        Ok(data.into_iter().map(|t| t.into_trade(sym.clone())).collect())
    }

    /// GET /api/v3/klines?symbol={}&interval={}&limit={}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let pair = unified_to_mexc(symbol);
        let limit = limit.min(1000);
        let url = format!(
            "{}/api/v3/klines?symbol={}&interval={}&limit={}",
            self.base_url,
            pair,
            interval_to_mexc_rest(interval),
            limit
        );
        let data: Vec<Vec<serde_json::Value>> = self.fetch(&url).await?;
        let sym = symbol.clone();
        let candles: Vec<Candle> = data
            .iter()
            .filter_map(|row| parse_kline_row(row, sym.clone()))
            .collect();
        Ok(candles)
    }

    /// GET /api/v3/ticker/24hr?symbol={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let pair = unified_to_mexc(symbol);
        let url = format!(
            "{}/api/v3/ticker/24hr?symbol={}",
            self.base_url, pair
        );
        let data: MexcTickerRaw = self.fetch(&url).await?;
        Ok(data.into_ticker())
    }

    /// GET /api/v3/ticker/24hr
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v3/ticker/24hr", self.base_url);
        let data: Vec<MexcTickerRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|t| t.into_ticker()).collect())
    }
}
