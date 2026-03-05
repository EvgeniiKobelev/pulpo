use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://fapi.xt.com";

pub struct XtFuturesRest {
    client: Client,
    base_url: String,
}

impl XtFuturesRest {
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

    /// Helper: GET request, return the unwrapped `result` field from XT
    /// response format `{"returnCode": 0, "result": ..., "error": {...}, "msgInfo": "..."}`.
    async fn get_result(&self, url: &str) -> Result<serde_json::Value> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::XtFutures,
                message: text,
                status: Some(status),
            });
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            }
        })?;

        // Check XT response code
        let code = json.get("returnCode").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = json
                .get("msgInfo")
                .and_then(|m| m.as_str())
                .or_else(|| {
                    json.get("error")
                        .and_then(|e| e.get("msg"))
                        .and_then(|m| m.as_str())
                })
                .unwrap_or("unknown error");
            return Err(GatewayError::Rest {
                exchange: ExchangeId::XtFutures,
                message: format!("returnCode={}, msg={}", code, msg),
                status: None,
            });
        }

        json.get("result")
            .cloned()
            .ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: "missing 'result' field in response".to_string(),
            })
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/future/market/v1/public/symbol/list", self.base_url);
        let data = self.get_result(&url).await?;
        let pairs: Vec<XtSymbolRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;

        let symbols = pairs
            .into_iter()
            .map(|p| p.into_symbol_info())
            .collect();

        Ok(ExchangeInfo {
            exchange: ExchangeId::XtFutures,
            symbols,
        })
    }

    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let xt_sym = unified_to_xt(symbol);
        let level = depth.min(50).max(1);
        let url = format!(
            "{}/future/market/v1/public/q/depth?symbol={}&level={}",
            self.base_url, xt_sym, level
        );
        let data = self.get_result(&url).await?;
        let raw: XtDepthRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_orderbook(symbol))
    }

    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let xt_sym = unified_to_xt(symbol);
        let num = limit.min(100).max(1);
        let url = format!(
            "{}/future/market/v1/public/q/deal?symbol={}&num={}",
            self.base_url, xt_sym, num
        );
        let data = self.get_result(&url).await?;
        let arr: Vec<XtTradeRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(arr
            .into_iter()
            .filter_map(|t| t.into_trade(symbol))
            .collect())
    }

    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let xt_sym = unified_to_xt(symbol);
        let bar = interval_to_xt(interval);
        let limit = limit.min(200);
        let url = format!(
            "{}/future/market/v1/public/q/kline?symbol={}&interval={}&limit={}",
            self.base_url, xt_sym, bar, limit
        );
        let data = self.get_result(&url).await?;
        let rows: Vec<XtKlineRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_candle(symbol, interval))
            .collect())
    }

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let xt_sym = unified_to_xt(symbol);
        let url = format!(
            "{}/future/market/v1/public/q/agg-ticker?symbol={}",
            self.base_url, xt_sym
        );
        let data = self.get_result(&url).await?;
        let raw: XtAggTickerRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_ticker(Some(symbol)))
    }

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/future/market/v1/public/q/agg-tickers", self.base_url);
        let data = self.get_result(&url).await?;
        let arr: Vec<XtAggTickerRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(arr.into_iter().map(|t| t.into_ticker(None)).collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let xt_sym = unified_to_xt(symbol);
        let url = format!(
            "{}/future/market/v1/public/q/funding-rate?symbol={}",
            self.base_url, xt_sym
        );
        let data = self.get_result(&url).await?;
        let raw: XtFundingRateRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_funding_rate(symbol))
    }

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        // Use agg-ticker which includes mark + index price
        let xt_sym = unified_to_xt(symbol);
        let url = format!(
            "{}/future/market/v1/public/q/agg-ticker?symbol={}",
            self.base_url, xt_sym
        );
        let data = self.get_result(&url).await?;
        let raw: XtAggTickerRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_mark_price(symbol))
    }

    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let xt_sym = unified_to_xt(symbol);
        let url = format!(
            "{}/future/market/v1/public/contract/open-interest?symbol={}",
            self.base_url, xt_sym
        );
        let data = self.get_result(&url).await?;
        let raw: XtOpenInterestRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::XtFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_open_interest(symbol))
    }
}
