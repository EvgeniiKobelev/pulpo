use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.bitget.com";

pub struct BitgetFuturesRest {
    client: Client,
    base_url: String,
}

impl BitgetFuturesRest {
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
                exchange: ExchangeId::BitgetFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BitgetFutures,
                message: body,
                status: Some(status),
            });
        }

        let wrapper: BitgetResponse<T> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BitgetFutures,
                message: e.to_string(),
            }
        })?;

        if wrapper.code != "00000" {
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BitgetFutures,
                message: format!("code={}: {}", wrapper.code, wrapper.msg),
                status: None,
            });
        }

        Ok(wrapper.data)
    }

    /// GET /api/v2/mix/market/contracts?productType=USDT-FUTURES
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!(
            "{}/api/v2/mix/market/contracts?productType=USDT-FUTURES",
            self.base_url
        );
        let result: Vec<BitgetMixContractRaw> = self.fetch(&url).await?;
        Ok(contracts_to_exchange_info(result))
    }

    /// GET /api/v2/mix/market/merge-depth?productType=USDT-FUTURES&symbol={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/mix/market/merge-depth?productType=USDT-FUTURES&symbol={}&limit={}",
            self.base_url, raw, depth
        );
        let result: BitgetMixOrderBookData = self.fetch(&url).await?;
        Ok(result.into_orderbook(symbol.clone()))
    }

    /// GET /api/v2/mix/market/fills?productType=USDT-FUTURES&symbol={}&limit={}
    ///
    /// Bitget futures limit max is 500.
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let limit = limit.min(500);
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/mix/market/fills?productType=USDT-FUTURES&symbol={}&limit={}",
            self.base_url, raw, limit
        );
        let result: Vec<BitgetMixTradeRaw> = self.fetch(&url).await?;
        Ok(result.into_iter().map(|t| t.into_trade()).collect())
    }

    /// GET /api/v2/mix/market/candles?productType=USDT-FUTURES&symbol={}&granularity={}&limit={}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/mix/market/candles?productType=USDT-FUTURES&symbol={}&granularity={}&limit={}",
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

    /// GET /api/v2/mix/market/ticker?productType=USDT-FUTURES&symbol={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<BitgetMixTickerRaw> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/mix/market/ticker?productType=USDT-FUTURES&symbol={}",
            self.base_url, raw
        );
        let result: Vec<BitgetMixTickerRaw> = self.fetch(&url).await?;
        result
            .into_iter()
            .next()
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::BitgetFutures,
                symbol: symbol.to_string(),
            })
    }

    /// GET /api/v2/mix/market/tickers?productType=USDT-FUTURES
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!(
            "{}/api/v2/mix/market/tickers?productType=USDT-FUTURES",
            self.base_url
        );
        let result: Vec<BitgetMixTickerRaw> = self.fetch(&url).await?;
        Ok(result.into_iter().map(|t| t.into_ticker()).collect())
    }

    /// GET /api/v2/mix/market/current-fund-rate?productType=USDT-FUTURES&symbol={}
    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/mix/market/current-fund-rate?productType=USDT-FUTURES&symbol={}",
            self.base_url, raw
        );
        let result: Vec<BitgetFundingRateRaw> = self.fetch(&url).await?;
        result
            .into_iter()
            .next()
            .map(|r| r.into_funding_rate())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::BitgetFutures,
                symbol: symbol.to_string(),
            })
    }

    /// GET /api/v2/mix/market/open-interest?productType=USDT-FUTURES&symbol={}
    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let raw = unified_to_bitget(symbol);
        let url = format!(
            "{}/api/v2/mix/market/open-interest?productType=USDT-FUTURES&symbol={}",
            self.base_url, raw
        );
        let result: Vec<BitgetOpenInterestRaw> = self.fetch(&url).await?;
        result
            .into_iter()
            .next()
            .map(|r| r.into_open_interest())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::BitgetFutures,
                symbol: symbol.to_string(),
            })
    }
}
