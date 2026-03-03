use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.toobit.com";

pub struct ToobitFuturesRest {
    client: Client,
    base_url: String,
}

impl ToobitFuturesRest {
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

    /// Helper: GET request, return parsed JSON.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::ToobitFutures,
                message: text,
                status: Some(status),
            });
        }

        resp.json::<serde_json::Value>().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            }
        })
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v1/exchangeInfo", self.base_url);
        let val = self.get_json(&url).await?;
        let resp: ToobitExchangeInfoResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;
        Ok(resp.into_exchange_info())
    }

    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let toobit_sym = unified_to_toobit(symbol);
        let limit = depth.min(1000);
        let url = format!(
            "{}/quote/v1/depth?symbol={}&limit={}",
            self.base_url, toobit_sym, limit
        );
        let val = self.get_json(&url).await?;
        let raw: ToobitOrderBookRaw =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_orderbook(symbol))
    }

    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let toobit_sym = unified_to_toobit(symbol);
        let limit = limit.min(60); // Toobit max is 60
        let url = format!(
            "{}/quote/v1/trades?symbol={}&limit={}",
            self.base_url, toobit_sym, limit
        );
        let val = self.get_json(&url).await?;
        let raw: Vec<ToobitTradeRaw> =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;
        Ok(raw
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
        let toobit_sym = unified_to_toobit(symbol);
        let bar = interval_to_toobit(interval);
        let limit = limit.min(1000);
        let url = format!(
            "{}/quote/v1/klines?symbol={}&interval={}&limit={}",
            self.base_url, toobit_sym, bar, limit
        );
        let val = self.get_json(&url).await?;
        let rows: Vec<Vec<serde_json::Value>> =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;
        Ok(rows
            .iter()
            .filter_map(|row| parse_kline_row(row, symbol, interval))
            .collect())
    }

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let toobit_sym = unified_to_toobit(symbol);
        let url = format!(
            "{}/quote/v1/contract/ticker/24hr?symbol={}",
            self.base_url, toobit_sym
        );
        let val = self.get_json(&url).await?;

        // The response might be a single object or wrapped in an array
        let raw: ToobitTickerRaw = if val.is_array() {
            let arr: Vec<ToobitTickerRaw> =
                serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                    exchange: ExchangeId::ToobitFutures,
                    message: e.to_string(),
                })?;
            arr.into_iter().next().ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: "empty ticker data".to_string(),
            })?
        } else {
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?
        };

        Ok(raw.into_ticker(Some(symbol)))
    }

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/quote/v1/contract/ticker/24hr", self.base_url);
        let val = self.get_json(&url).await?;
        let raw: Vec<ToobitTickerRaw> =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_iter().map(|t| t.into_ticker(None)).collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let toobit_sym = unified_to_toobit(symbol);
        let url = format!(
            "{}/api/v1/futures/fundingRate?symbol={}",
            self.base_url, toobit_sym
        );
        let val = self.get_json(&url).await?;

        // Response may be a single object or an array
        let raw: ToobitFundingRateRaw = if val.is_array() {
            let arr: Vec<ToobitFundingRateRaw> =
                serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                    exchange: ExchangeId::ToobitFutures,
                    message: e.to_string(),
                })?;
            arr.into_iter().next().ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: "empty funding rate data".to_string(),
            })?
        } else {
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?
        };

        Ok(raw.into_funding_rate())
    }

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let toobit_sym = unified_to_toobit(symbol);
        let url = format!(
            "{}/quote/v1/markPrice?symbol={}",
            self.base_url, toobit_sym
        );
        let val = self.get_json(&url).await?;
        let raw: ToobitMarkPriceRaw =
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_mark_price(symbol))
    }

    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        // Toobit does not have a dedicated open interest REST endpoint.
        // We approximate from the ticker volume data.
        let toobit_sym = unified_to_toobit(symbol);
        let url = format!(
            "{}/quote/v1/contract/ticker/24hr?symbol={}",
            self.base_url, toobit_sym
        );
        let val = self.get_json(&url).await?;

        let raw: ToobitTickerRaw = if val.is_array() {
            let arr: Vec<ToobitTickerRaw> =
                serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                    exchange: ExchangeId::ToobitFutures,
                    message: e.to_string(),
                })?;
            arr.into_iter().next().ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: "empty ticker data for open interest".to_string(),
            })?
        } else {
            serde_json::from_value(val).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::ToobitFutures,
                message: e.to_string(),
            })?
        };

        let last_price = raw
            .c
            .as_deref()
            .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
            .unwrap_or_default();
        let vol = raw
            .v
            .as_deref()
            .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
            .unwrap_or_default();

        Ok(OpenInterest {
            exchange: ExchangeId::ToobitFutures,
            symbol: symbol.clone(),
            open_interest: vol,
            open_interest_value: vol * last_price,
            timestamp_ms: raw.t.unwrap_or(0),
        })
    }
}
