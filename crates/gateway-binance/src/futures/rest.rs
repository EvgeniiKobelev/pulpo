use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://fapi.binance.com";

pub struct BinanceFuturesRest {
    client: Client,
    base_url: String,
}

impl BinanceFuturesRest {
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
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: BinanceFuturesExchangeInfoRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
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
            unified_to_binance(symbol),
            depth
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: BinanceFuturesOrderBookRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
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
            unified_to_binance(symbol),
            limit
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: Vec<BinanceFuturesTradeRaw> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
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
            unified_to_binance(symbol),
            interval_to_binance(interval),
            limit
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let rows: Vec<Vec<serde_json::Value>> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
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
            unified_to_binance(symbol)
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: BinanceFuturesTickerRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
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
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: Vec<BinanceFuturesTickerRaw> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw.into_iter().map(|t| t.into_ticker()).collect())
    }

    /// GET /fapi/v1/premiumIndex?symbol={}
    pub async fn premium_index(&self, symbol: &Symbol) -> Result<BinancePremiumIndexRaw> {
        let url = format!(
            "{}/fapi/v1/premiumIndex?symbol={}",
            self.base_url,
            unified_to_binance(symbol)
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: BinancePremiumIndexRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw)
    }

    /// GET /fapi/v1/openInterest?symbol={}
    pub async fn open_interest(&self, symbol: &Symbol) -> Result<BinanceOpenInterestRaw> {
        let url = format!(
            "{}/fapi/v1/openInterest?symbol={}",
            self.base_url,
            unified_to_binance(symbol)
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: BinanceOpenInterestRaw = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw)
    }

    /// GET /fapi/v1/allForceOrders?symbol={}&limit={}
    pub async fn force_orders(
        &self,
        symbol: &Symbol,
        limit: u16,
    ) -> Result<Vec<BinanceForceOrderRaw>> {
        let url = format!(
            "{}/fapi/v1/allForceOrders?symbol={}&limit={}",
            self.base_url,
            unified_to_binance(symbol),
            limit
        );
        let resp = self.client.get(&url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BinanceFutures,
                message: body,
                status: Some(status),
            });
        }

        let raw: Vec<BinanceForceOrderRaw> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BinanceFutures,
                message: e.to_string(),
            }
        })?;
        Ok(raw)
    }
}
