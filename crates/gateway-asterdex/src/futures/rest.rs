use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://fapi.asterdex.com";

pub struct AsterdexFuturesRest {
    client: Client,
    base_url: String,
}

impl AsterdexFuturesRest {
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

    /// GET /fapi/v1/exchangeInfo
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: AsterdexExchangeInfoRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw.into_exchange_info())
    }

    /// GET /fapi/v1/depth?symbol={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let url = format!(
            "{}/fapi/v1/depth?symbol={}&limit={}",
            self.base_url,
            unified_to_asterdex(symbol),
            depth
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: AsterdexOrderBookRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /fapi/v1/trades?symbol={}&limit={}
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let url = format!(
            "{}/fapi/v1/trades?symbol={}&limit={}",
            self.base_url,
            unified_to_asterdex(symbol),
            limit
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: Vec<AsterdexTradeRaw> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw
            .into_iter()
            .map(|t| t.into_trade(symbol.clone()))
            .collect())
    }

    /// GET /fapi/v1/klines?symbol={}&interval={}&limit={}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let url = format!(
            "{}/fapi/v1/klines?symbol={}&interval={}&limit={}",
            self.base_url,
            unified_to_asterdex(symbol),
            interval_to_asterdex(interval),
            limit
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let rows: Vec<Vec<serde_json::Value>> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(rows
            .iter()
            .filter_map(|row| parse_kline_row(row, symbol.clone()))
            .collect())
    }

    /// GET /fapi/v1/ticker/24hr?symbol={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let url = format!(
            "{}/fapi/v1/ticker/24hr?symbol={}",
            self.base_url,
            unified_to_asterdex(symbol)
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: AsterdexTickerRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw.into_ticker())
    }

    /// GET /fapi/v1/ticker/24hr (all tickers)
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/fapi/v1/ticker/24hr", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: Vec<AsterdexTickerRaw> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw.into_iter().map(|t| t.into_ticker()).collect())
    }

    /// GET /fapi/v1/premiumIndex?symbol={}
    pub async fn premium_index(&self, symbol: &Symbol) -> Result<AsterdexPremiumIndexRaw> {
        let url = format!(
            "{}/fapi/v1/premiumIndex?symbol={}",
            self.base_url,
            unified_to_asterdex(symbol)
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: AsterdexPremiumIndexRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw)
    }

    /// GET /fapi/v1/openInterest?symbol={}
    pub async fn open_interest(&self, symbol: &Symbol) -> Result<AsterdexOpenInterestRaw> {
        let url = format!(
            "{}/fapi/v1/openInterest?symbol={}",
            self.base_url,
            unified_to_asterdex(symbol)
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: AsterdexOpenInterestRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw)
    }

    /// GET /fapi/v1/allForceOrders?symbol={}&limit={}
    ///
    /// Note: This endpoint may be unavailable on Asterdex ("out of maintenance").
    /// Returns an empty vec if the server rejects the request.
    pub async fn force_orders(
        &self,
        symbol: &Symbol,
        limit: u16,
    ) -> Result<Vec<AsterdexForceOrderRaw>> {
        let url = format!(
            "{}/fapi/v1/allForceOrders?symbol={}&limit={}",
            self.base_url,
            unified_to_asterdex(symbol),
            limit
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            // Endpoint may be disabled — return empty instead of erroring.
            return Ok(vec![]);
        }

        let raw: Vec<AsterdexForceOrderRaw> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::AsterdexFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw)
    }
}
