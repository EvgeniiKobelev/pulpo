use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://openapi.blofin.com";

pub struct BlofinFuturesRest {
    client: Client,
    base_url: String,
}

impl BlofinFuturesRest {
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
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BlofinFutures,
                message: text,
                status: Some(status),
            });
        }

        let val: serde_json::Value = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            }
        })?;

        // BloFin returns code "0" for success
        if let Some(code) = val.get("code").and_then(|c| c.as_str()) {
            if code != "0" {
                let msg = val
                    .get("msg")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                return Err(GatewayError::Rest {
                    exchange: ExchangeId::BlofinFutures,
                    message: format!("code={}, msg={}", code, msg),
                    status: None,
                });
            }
        }

        Ok(val)
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v1/market/instruments", self.base_url);
        let val = self.get_json(&url).await?;
        let resp: BlofinInstrumentsResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        Ok(resp.into_exchange_info())
    }

    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let inst_id = unified_to_blofin(symbol);
        let size = depth.min(100); // BloFin max depth is 100
        let url = format!(
            "{}/api/v1/market/books?instId={}&size={}",
            self.base_url, inst_id, size
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinOrderbookResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        resp.data
            .into_iter()
            .next()
            .map(|d| d.into_orderbook(&inst_id))
            .ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: "empty orderbook data".to_string(),
            })
    }

    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let inst_id = unified_to_blofin(symbol);
        let url = format!(
            "{}/api/v1/market/trades?instId={}&limit={}",
            self.base_url, inst_id, limit
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinTradesResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        Ok(resp
            .data
            .into_iter()
            .filter_map(|t| t.into_trade())
            .collect())
    }

    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let inst_id = unified_to_blofin(symbol);
        let bar = interval_to_blofin(interval);
        let limit = limit.min(100); // BloFin max candles per request is 100
        let url = format!(
            "{}/api/v1/market/candles?instId={}&bar={}&limit={}",
            self.base_url, inst_id, bar, limit
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinCandlesResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        Ok(resp.into_candles(&inst_id, interval))
    }

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let inst_id = unified_to_blofin(symbol);
        let url = format!(
            "{}/api/v1/market/tickers?instId={}",
            self.base_url, inst_id
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinTickersResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        resp.data
            .into_iter()
            .next()
            .map(|t| t.into_ticker())
            .ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: "empty ticker data".to_string(),
            })
    }

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v1/market/tickers", self.base_url);
        let val = self.get_json(&url).await?;
        let resp: BlofinTickersResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        Ok(resp.data.into_iter().map(|t| t.into_ticker()).collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let inst_id = unified_to_blofin(symbol);
        let url = format!(
            "{}/api/v1/market/funding-rate?instId={}",
            self.base_url, inst_id
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinFundingRateResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        resp.data
            .into_iter()
            .next()
            .map(|d| d.into_funding_rate())
            .ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: "empty funding rate data".to_string(),
            })
    }

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let inst_id = unified_to_blofin(symbol);
        let url = format!(
            "{}/api/v1/market/mark-price?instId={}",
            self.base_url, inst_id
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinMarkPriceResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        resp.data
            .into_iter()
            .next()
            .map(|d| d.into_mark_price())
            .ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: "empty mark price data".to_string(),
            })
    }

    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        // BloFin ticker endpoint includes mark price which we can use.
        // For open interest, we extract it from the ticker.
        let inst_id = unified_to_blofin(symbol);
        let url = format!(
            "{}/api/v1/market/tickers?instId={}",
            self.base_url, inst_id
        );
        let val = self.get_json(&url).await?;
        let resp: BlofinTickersResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: e.to_string(),
            })?;
        let ticker = resp.data.into_iter().next().ok_or_else(|| {
            GatewayError::Parse {
                exchange: ExchangeId::BlofinFutures,
                message: "empty ticker data for open interest".to_string(),
            }
        })?;

        let symbol = blofin_symbol_to_unified(&ticker.inst_id);
        let last_price = ticker
            .last
            .as_deref()
            .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
            .unwrap_or_default();
        let vol = ticker
            .vol_currency_24h
            .as_deref()
            .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
            .unwrap_or_default();
        let ts = ticker
            .ts
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        Ok(OpenInterest {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            open_interest: vol,
            open_interest_value: vol * last_price,
            timestamp_ms: ts,
        })
    }
}
