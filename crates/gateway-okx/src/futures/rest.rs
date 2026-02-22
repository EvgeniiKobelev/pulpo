use gateway_core::*;
use reqwest::Client;

use crate::futures::mapper::*;

const BASE_URL: &str = "https://www.okx.com";
const EXCHANGE: ExchangeId = ExchangeId::Okx;

pub struct OkxSwapRest {
    client: Client,
    base_url: String,
}

impl OkxSwapRest {
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

    async fn fetch<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<Vec<T>> {
        let resp = self.client.get(url).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: EXCHANGE,
                message: e.to_string(),
                status: e.status().map(|s| s.as_u16()),
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("HTTP {status}: {body}"),
                status: Some(status),
            });
        }

        let wrapper: OkxResponse<T> = resp.json().await.map_err(|e| GatewayError::Parse {
            exchange: EXCHANGE,
            message: e.to_string(),
        })?;

        if wrapper.code != "0" {
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("code={}: {}", wrapper.code, wrapper.msg),
                status: None,
            });
        }

        Ok(wrapper.data)
    }

    // ----- Exchange Info (SWAP instruments) -----

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v5/public/instruments?instType=SWAP", self.base_url);
        let data: Vec<OkxSwapInstrumentRaw> = self.fetch(&url).await?;
        let symbols = data
            .into_iter()
            .filter(|r| r.ct_type == "linear")
            .filter_map(|r| r.into_symbol_info(EXCHANGE))
            .collect();
        Ok(ExchangeInfo {
            exchange: EXCHANGE,
            symbols,
        })
    }

    // ----- Order Book -----

    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let inst_id = unified_to_okx_swap(symbol);
        let url = format!(
            "{}/api/v5/market/books?instId={}&sz={}",
            self.base_url, inst_id, depth
        );
        let data: Vec<OkxWsBookData> = self.fetch(&url).await?;
        let raw = data.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: EXCHANGE,
            message: "empty orderbook response".into(),
        })?;
        Ok(OrderBook {
            exchange: EXCHANGE,
            symbol: symbol.clone(),
            bids: parse_levels(&raw.bids),
            asks: parse_levels(&raw.asks),
            timestamp_ms: raw.ts.parse().unwrap_or(0),
            sequence: raw.seq_id,
        })
    }

    // ----- Trades -----

    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let inst_id = unified_to_okx_swap(symbol);
        let limit = limit.min(500);
        let url = format!(
            "{}/api/v5/market/trades?instId={}&limit={}",
            self.base_url, inst_id, limit
        );
        let data: Vec<OkxSwapTradeRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|r| r.into_trade(EXCHANGE)).collect())
    }

    // ----- Candles -----

    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let inst_id = unified_to_okx_swap(symbol);
        let bar = interval_to_okx(interval);
        let limit = limit.min(100);
        let url = format!(
            "{}/api/v5/market/candles?instId={}&bar={}&limit={}",
            self.base_url, inst_id, bar, limit
        );
        let data: Vec<Vec<String>> = self.fetch(&url).await?;
        let mut candles: Vec<Candle> = data
            .iter()
            .filter_map(|row| parse_kline_row(row, EXCHANGE, symbol, interval))
            .collect();
        // OKX returns newest-first
        candles.reverse();
        Ok(candles)
    }

    // ----- Ticker -----

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let inst_id = unified_to_okx_swap(symbol);
        let url = format!(
            "{}/api/v5/market/ticker?instId={}",
            self.base_url, inst_id
        );
        let data: Vec<OkxSwapTickerRaw> = self.fetch(&url).await?;
        let raw = data.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: EXCHANGE,
            message: "empty ticker response".into(),
        })?;
        Ok(raw.into_ticker(EXCHANGE))
    }

    // ----- All Tickers -----

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!(
            "{}/api/v5/market/tickers?instType=SWAP",
            self.base_url
        );
        let data: Vec<OkxSwapTickerRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|r| r.into_ticker(EXCHANGE)).collect())
    }

    // ----- Funding Rate -----

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let inst_id = unified_to_okx_swap(symbol);
        let url = format!(
            "{}/api/v5/public/funding-rate?instId={}",
            self.base_url, inst_id
        );
        let data: Vec<OkxFundingRateRaw> = self.fetch(&url).await?;
        let raw = data.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: EXCHANGE,
            message: "empty funding rate response".into(),
        })?;
        Ok(raw.into_funding_rate(EXCHANGE))
    }

    // ----- Mark Price -----

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let inst_id = unified_to_okx_swap(symbol);
        let url = format!(
            "{}/api/v5/public/mark-price?instType=SWAP&instId={}",
            self.base_url, inst_id
        );
        let data: Vec<OkxMarkPriceRaw> = self.fetch(&url).await?;
        let raw = data.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: EXCHANGE,
            message: "empty mark price response".into(),
        })?;
        Ok(raw.into_mark_price(EXCHANGE))
    }

    // ----- Open Interest -----

    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let inst_id = unified_to_okx_swap(symbol);
        let url = format!(
            "{}/api/v5/public/open-interest?instType=SWAP&instId={}",
            self.base_url, inst_id
        );
        let data: Vec<OkxOpenInterestRaw> = self.fetch(&url).await?;
        let raw = data.into_iter().next().ok_or_else(|| GatewayError::Parse {
            exchange: EXCHANGE,
            message: "empty open interest response".into(),
        })?;
        Ok(raw.into_open_interest(EXCHANGE))
    }

    // ----- Liquidations -----

    pub async fn liquidations(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Liquidation>> {
        let inst_id = unified_to_okx_swap(symbol);
        let url = format!(
            "{}/api/v5/public/liquidation-orders?instType=SWAP&instId={}&state=filled&limit={}",
            self.base_url, inst_id, limit
        );
        let data: Vec<OkxLiquidationOrderRaw> = self.fetch(&url).await?;
        Ok(data
            .into_iter()
            .flat_map(|r| r.into_liquidations(EXCHANGE))
            .collect())
    }
}
