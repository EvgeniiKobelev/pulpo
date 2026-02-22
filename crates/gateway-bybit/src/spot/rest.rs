use gateway_core::*;
use crate::spot::mapper::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.bybit.com";

pub struct BybitRest {
    client: Client,
    base_url: String,
}

impl BybitRest {
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

    /// Helper to handle the Bybit response wrapper.
    ///
    /// All Bybit V5 responses are wrapped in `{"retCode":0,"retMsg":"OK","result":{...}}`.
    /// We check the HTTP status first, then parse the wrapper, then check `retCode == 0`.
    async fn fetch<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BybitSpot,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BybitSpot,
                message: body,
                status: Some(status),
            });
        }

        let wrapper: BybitResponse<T> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BybitSpot,
                message: e.to_string(),
            }
        })?;

        if wrapper.ret_code != 0 {
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BybitSpot,
                message: format!("retCode={}: {}", wrapper.ret_code, wrapper.ret_msg),
                status: None,
            });
        }

        Ok(wrapper.result)
    }

    /// GET /v5/market/instruments-info?category=spot
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/v5/market/instruments-info?category=spot", self.base_url);
        let result: BybitInstrumentsResult = self.fetch(&url).await?;
        Ok(result.into_exchange_info())
    }

    /// GET /v5/market/orderbook?category=spot&symbol={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let url = format!(
            "{}/v5/market/orderbook?category=spot&symbol={}&limit={}",
            self.base_url,
            unified_to_bybit(symbol),
            depth
        );
        let result: BybitOrderBookResult = self.fetch(&url).await?;
        Ok(result.into_orderbook())
    }

    /// GET /v5/market/recent-trade?category=spot&symbol={}&limit={}
    ///
    /// Bybit spot limit max is 60.
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let limit = limit.min(60); // Bybit spot limit
        let url = format!(
            "{}/v5/market/recent-trade?category=spot&symbol={}&limit={}",
            self.base_url,
            unified_to_bybit(symbol),
            limit
        );
        let result: BybitTradesResult = self.fetch(&url).await?;
        Ok(result.list.into_iter().map(|t| t.into_trade()).collect())
    }

    /// GET /v5/market/kline?category=spot&symbol={}&interval={}&limit={}
    ///
    /// Bybit returns klines in REVERSE order (newest first), so we reverse them.
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let url = format!(
            "{}/v5/market/kline?category=spot&symbol={}&interval={}&limit={}",
            self.base_url,
            unified_to_bybit(symbol),
            interval_to_bybit(interval),
            limit
        );
        let result: BybitKlinesResult = self.fetch(&url).await?;
        let sym = symbol.clone();
        let mut candles: Vec<Candle> = result
            .list
            .iter()
            .filter_map(|row| parse_kline_row(row, sym.clone()))
            .collect();
        candles.reverse(); // Bybit returns newest first
        Ok(candles)
    }

    /// GET /v5/market/tickers?category=spot&symbol={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let url = format!(
            "{}/v5/market/tickers?category=spot&symbol={}",
            self.base_url,
            unified_to_bybit(symbol)
        );
        let result: BybitTickersResult = self.fetch(&url).await?;
        result
            .list
            .into_iter()
            .next()
            .map(|t| t.into_ticker())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::BybitSpot,
                symbol: symbol.to_string(),
            })
    }

    /// GET /v5/market/tickers?category=spot
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/v5/market/tickers?category=spot", self.base_url);
        let result: BybitTickersResult = self.fetch(&url).await?;
        Ok(result.list.into_iter().map(|t| t.into_ticker()).collect())
    }
}
