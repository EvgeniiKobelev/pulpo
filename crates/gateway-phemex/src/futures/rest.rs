use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.phemex.com";

pub struct PhemexFuturesRest {
    client: Client,
    base_url: String,
}

impl PhemexFuturesRest {
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

    /// Helper: GET request with error handling.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::PhemexFutures,
                message: text,
                status: Some(status),
            });
        }

        resp.json::<serde_json::Value>().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            }
        })
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/public/products", self.base_url);
        let val = self.get_json(&url).await?;
        let resp: PhemexProductsResponse = serde_json::from_value(val).map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            }
        })?;
        if resp.code != 0 {
            return Err(GatewayError::Rest {
                exchange: ExchangeId::PhemexFutures,
                message: format!("code={}, msg={}", resp.code, resp.msg),
                status: None,
            });
        }
        Ok(resp.data.into_exchange_info())
    }

    pub async fn orderbook(&self, symbol: &Symbol, _depth: u16) -> Result<OrderBook> {
        let url = format!(
            "{}/md/v2/orderbook?symbol={}",
            self.base_url,
            unified_to_phemex(symbol)
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexOrderbookResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        Ok(resp.result.into_orderbook())
    }

    pub async fn trades(&self, symbol: &Symbol, _limit: u16) -> Result<Vec<Trade>> {
        let url = format!(
            "{}/md/v2/trade?symbol={}",
            self.base_url,
            unified_to_phemex(symbol)
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexTradesResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        let sym_str = resp.result.symbol.clone();
        Ok(resp
            .result
            .trades_p
            .into_iter()
            .filter_map(|t| t.into_trade(&sym_str))
            .collect())
    }

    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let now_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let interval_s = interval.as_secs();
        let from_s = now_s.saturating_sub(interval_s * limit as u64);

        let url = format!(
            "{}/md/v2/kline?symbol={}&interval={}&from={}&to={}",
            self.base_url,
            unified_to_phemex(symbol),
            interval_s,
            from_s,
            now_s,
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexKlineResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        let sym = phemex_symbol_to_unified(&unified_to_phemex(symbol));
        Ok(resp
            .result
            .into_candles()
            .into_iter()
            .map(|mut c| {
                c.symbol = sym.clone();
                c
            })
            .collect())
    }

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let url = format!(
            "{}/md/v2/ticker/24hr?symbol={}",
            self.base_url,
            unified_to_phemex(symbol)
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexTickerResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        Ok(resp.result.into_ticker())
    }

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/md/v2/ticker/24hr/all", self.base_url);
        let val = self.get_json(&url).await?;
        let resp: PhemexAllTickersResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        Ok(resp.result.into_iter().map(|t| t.into_ticker()).collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let url = format!(
            "{}/md/v2/ticker/24hr?symbol={}",
            self.base_url,
            unified_to_phemex(symbol)
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexTickerResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        Ok(resp.result.into_funding_rate())
    }

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let url = format!(
            "{}/md/v2/ticker/24hr?symbol={}",
            self.base_url,
            unified_to_phemex(symbol)
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexTickerResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        Ok(resp.result.into_mark_price())
    }

    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let url = format!(
            "{}/md/v2/ticker/24hr?symbol={}",
            self.base_url,
            unified_to_phemex(symbol)
        );
        let val = self.get_json(&url).await?;
        let resp: PhemexTickerResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::PhemexFutures,
                message: e.to_string(),
            })?;
        Ok(resp.result.into_open_interest())
    }
}
