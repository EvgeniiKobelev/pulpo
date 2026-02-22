use crate::spot::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.bitget.com";

pub struct BitgetRest {
    client: Client,
    base_url: String,
}

impl BitgetRest {
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

    /// Helper to handle the Bitget response wrapper.
    ///
    /// All Bitget V2 responses are wrapped in `{"code":"00000","msg":"success","data":{...}}`.
    /// We check the HTTP status first, then parse the wrapper, then check `code == "00000"`.
    async fn fetch<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::BitgetSpot,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BitgetSpot,
                message: body,
                status: Some(status),
            });
        }

        let wrapper: BitgetResponse<T> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BitgetSpot,
                message: e.to_string(),
            }
        })?;

        if wrapper.code != "00000" {
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BitgetSpot,
                message: format!("code={}: {}", wrapper.code, wrapper.msg),
                status: None,
            });
        }

        Ok(wrapper.data)
    }

    /// GET /api/v2/spot/public/symbols
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v2/spot/public/symbols", self.base_url);
        let result: Vec<BitgetSymbolRaw> = self.fetch(&url).await?;
        Ok(symbols_to_exchange_info(result))
    }

    /// GET /api/v2/spot/market/orderbook?symbol={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/spot/market/orderbook?symbol={}&limit={}",
            self.base_url, raw, depth
        );
        let result: BitgetOrderBookData = self.fetch(&url).await?;
        Ok(result.into_orderbook(symbol.clone()))
    }

    /// GET /api/v2/spot/market/fills?symbol={}&limit={}
    ///
    /// Bitget spot limit max is 500.
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let limit = limit.min(500);
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/spot/market/fills?symbol={}&limit={}",
            self.base_url, raw, limit
        );
        let result: Vec<BitgetTradeRaw> = self.fetch(&url).await?;
        Ok(result.into_iter().map(|t| t.into_trade()).collect())
    }

    /// GET /api/v2/spot/market/candles?symbol={}&granularity={}&limit={}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/spot/market/candles?symbol={}&granularity={}&limit={}",
            self.base_url,
            raw,
            interval_to_bitget_rest(interval),
            limit
        );
        let result: Vec<Vec<String>> = self.fetch(&url).await?;
        let sym = symbol.clone();
        let mut candles: Vec<Candle> = result
            .iter()
            .filter_map(|row| parse_kline_row(row, sym.clone()))
            .collect();
        candles.reverse(); // Bitget returns newest first
        Ok(candles)
    }

    /// GET /api/v2/spot/market/tickers?symbol={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/spot/market/tickers?symbol={}",
            self.base_url, raw
        );
        let result: Vec<BitgetTickerRaw> = self.fetch(&url).await?;
        result
            .into_iter()
            .next()
            .map(|t| t.into_ticker())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::BitgetSpot,
                symbol: symbol.to_string(),
            })
    }

    /// GET /api/v2/spot/market/tickers
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v2/spot/market/tickers", self.base_url);
        let result: Vec<BitgetTickerRaw> = self.fetch(&url).await?;
        Ok(result.into_iter().map(|t| t.into_ticker()).collect())
    }
}
