use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://fapi.bitunix.com";

pub struct BitunixFuturesRest {
    client: Client,
    base_url: String,
}

impl BitunixFuturesRest {
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

    /// Helper: GET request, return the unwrapped `data` field from Bitunix
    /// response format `{"code": 0, "data": ..., "msg": "..."}`.
    async fn get_data(&self, url: &str) -> Result<serde_json::Value> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BitunixFutures,
                message: text,
                status: Some(status),
            });
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            }
        })?;

        // Check Bitunix response code
        let code = json.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = json
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BitunixFutures,
                message: format!("code={}, msg={}", code, msg),
                status: None,
            });
        }

        json.get("data")
            .cloned()
            .ok_or_else(|| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: "missing 'data' field in response".to_string(),
            })
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v1/futures/market/trading_pairs", self.base_url);
        let data = self.get_data(&url).await?;
        let pairs: Vec<BitunixTradingPairRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;

        let symbols = pairs
            .into_iter()
            .map(|p| p.into_symbol_info())
            .collect();

        Ok(ExchangeInfo {
            exchange: ExchangeId::BitunixFutures,
            symbols,
        })
    }

    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let bitunix_sym = unified_to_bitunix(symbol);
        let limit = match depth {
            0..=1 => "1",
            2..=5 => "5",
            6..=15 => "15",
            16..=50 => "50",
            _ => "max",
        };
        let url = format!(
            "{}/api/v1/futures/market/depth?symbol={}&limit={}",
            self.base_url, bitunix_sym, limit
        );
        let data = self.get_data(&url).await?;
        let raw: BitunixDepthRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_orderbook(symbol))
    }

    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let bitunix_sym = unified_to_bitunix(symbol);
        let bar = interval_to_bitunix(interval);
        let limit = limit.min(200); // Bitunix max is 200
        let url = format!(
            "{}/api/v1/futures/market/kline?symbol={}&interval={}&limit={}",
            self.base_url, bitunix_sym, bar, limit
        );
        let data = self.get_data(&url).await?;
        let rows: Vec<BitunixKlineRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_candle(symbol, interval))
            .collect())
    }

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let bitunix_sym = unified_to_bitunix(symbol);
        let url = format!(
            "{}/api/v1/futures/market/tickers?symbols={}",
            self.base_url, bitunix_sym
        );
        let data = self.get_data(&url).await?;
        let arr: Vec<BitunixTickerRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;
        let raw = arr.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: ExchangeId::BitunixFutures,
            message: "empty ticker data".to_string(),
        })?;
        Ok(raw.into_ticker(Some(symbol)))
    }

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v1/futures/market/tickers", self.base_url);
        let data = self.get_data(&url).await?;
        let arr: Vec<BitunixTickerRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;
        Ok(arr.into_iter().map(|t| t.into_ticker(None)).collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let bitunix_sym = unified_to_bitunix(symbol);
        let url = format!(
            "{}/api/v1/futures/market/funding_rate?symbol={}",
            self.base_url, bitunix_sym
        );
        let data = self.get_data(&url).await?;
        // data is a single object, not an array
        let raw: BitunixFundingRateRaw =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;
        Ok(raw.into_funding_rate(symbol))
    }

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        // Bitunix includes mark price in the tickers endpoint.
        let bitunix_sym = unified_to_bitunix(symbol);
        let url = format!(
            "{}/api/v1/futures/market/tickers?symbols={}",
            self.base_url, bitunix_sym
        );
        let data = self.get_data(&url).await?;
        let arr: Vec<BitunixTickerRaw> =
            serde_json::from_value(data).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::BitunixFutures,
                message: e.to_string(),
            })?;
        let raw = arr.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: ExchangeId::BitunixFutures,
            message: "empty ticker data for mark price".to_string(),
        })?;
        Ok(raw.into_mark_price(symbol))
    }
}
