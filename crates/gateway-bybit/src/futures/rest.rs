use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.bybit.com";

/// Max page size accepted by `GET /v5/market/instruments-info`.
const INSTRUMENTS_PAGE_LIMIT: u16 = 1000;

/// Safety cap on `instruments-info` pagination.
///
/// At 1000 instruments per page this is far above the real instrument count;
/// it only guards against an endless loop if the API ever repeats a cursor.
const INSTRUMENTS_MAX_PAGES: usize = 10;

pub struct BybitLinearRest {
    client: Client,
    base_url: String,
}

impl BybitLinearRest {
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
                exchange: ExchangeId::BybitFutures,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BybitFutures,
                message: body,
                status: Some(status),
            });
        }

        let wrapper: BybitResponse<T> = resp.json().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::BybitFutures,
                message: e.to_string(),
            }
        })?;

        if wrapper.ret_code != 0 {
            return Err(GatewayError::Rest {
                exchange: ExchangeId::BybitFutures,
                message: format!("retCode={}: {}", wrapper.ret_code, wrapper.ret_msg),
                status: None,
            });
        }

        Ok(wrapper.result)
    }

    /// GET /v5/market/instruments-info?category=linear&limit=1000[&cursor=...]
    ///
    /// The endpoint is paginated: without `limit` it returns only 500 entries
    /// while Bybit lists ~850 linear instruments, silently truncating the list
    /// (SOLUSDT, XRPUSDT and friends went missing). We request the maximum page
    /// size and follow `nextPageCursor` until it comes back empty or absent,
    /// capped at [`INSTRUMENTS_MAX_PAGES`] pages. The cursor is already
    /// percent-encoded by the API and is passed through as-is.
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let mut category = String::new();
        let mut list = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0usize;

        for _ in 0..INSTRUMENTS_MAX_PAGES {
            let mut url = format!(
                "{}/v5/market/instruments-info?category=linear&limit={}",
                self.base_url, INSTRUMENTS_PAGE_LIMIT
            );
            if let Some(c) = &cursor {
                url.push_str("&cursor=");
                url.push_str(c);
            }

            let page: BybitLinearInstrumentsResult = self.fetch(&url).await?;
            pages += 1;
            if category.is_empty() {
                category = page.category;
            }
            list.extend(page.list);

            match page.next_page_cursor {
                Some(next) if !next.is_empty() => cursor = Some(next),
                _ => {
                    cursor = None;
                    break;
                }
            }
        }

        if cursor.is_some() {
            tracing::warn!(
                pages,
                instruments = list.len(),
                "instruments-info: page cap reached, list may be truncated"
            );
        }

        tracing::debug!(
            exchange = "bybit_futures",
            pages,
            instruments = list.len(),
            "fetched instruments-info"
        );

        let result = BybitLinearInstrumentsResult {
            category,
            list,
            next_page_cursor: None,
        };
        Ok(result.into_exchange_info())
    }

    /// GET /v5/market/orderbook?category=linear&symbol={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let url = format!(
            "{}/v5/market/orderbook?category=linear&symbol={}&limit={}",
            self.base_url,
            unified_to_bybit(symbol),
            depth
        );
        let result: BybitLinearOrderBookResult = self.fetch(&url).await?;
        Ok(result.into_orderbook())
    }

    /// GET /v5/market/recent-trade?category=linear&symbol={}&limit={}
    ///
    /// Bybit linear limit max is 1000.
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let limit = limit.min(1000); // Bybit linear limit
        let url = format!(
            "{}/v5/market/recent-trade?category=linear&symbol={}&limit={}",
            self.base_url,
            unified_to_bybit(symbol),
            limit
        );
        let result: BybitLinearTradesResult = self.fetch(&url).await?;
        Ok(result.list.into_iter().map(|t| t.into_trade()).collect())
    }

    /// GET /v5/market/kline?category=linear&symbol={}&interval={}&limit={}
    ///
    /// Bybit returns klines in REVERSE order (newest first), so we reverse them.
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let url = format!(
            "{}/v5/market/kline?category=linear&symbol={}&interval={}&limit={}",
            self.base_url,
            unified_to_bybit(symbol),
            interval_to_bybit(interval),
            limit
        );
        let result: BybitLinearKlinesResult = self.fetch(&url).await?;
        let sym = symbol.clone();
        let mut candles: Vec<Candle> = result
            .list
            .iter()
            .filter_map(|row| parse_kline_row(row, sym.clone()))
            .collect();
        candles.reverse(); // Bybit returns newest first
        Ok(candles)
    }

    /// GET /v5/market/tickers?category=linear&symbol={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<BybitLinearTickerRaw> {
        let url = format!(
            "{}/v5/market/tickers?category=linear&symbol={}",
            self.base_url,
            unified_to_bybit(symbol)
        );
        let result: BybitLinearTickersResult = self.fetch(&url).await?;
        result
            .list
            .into_iter()
            .next()
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: ExchangeId::BybitFutures,
                symbol: symbol.to_string(),
            })
    }

    /// GET /v5/market/tickers?category=linear
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/v5/market/tickers?category=linear", self.base_url);
        let result: BybitLinearTickersResult = self.fetch(&url).await?;
        Ok(result.list.into_iter().map(|t| t.into_ticker()).collect())
    }

    /// GET /v5/market/funding/history?category=linear&symbol={}&limit=1
    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<BybitFundingHistoryRaw> {
        let url = format!(
            "{}/v5/market/funding/history?category=linear&symbol={}&limit=1",
            self.base_url,
            unified_to_bybit(symbol)
        );
        let result: BybitFundingHistoryResult = self.fetch(&url).await?;
        result
            .list
            .into_iter()
            .next()
            .ok_or_else(|| GatewayError::Rest {
                exchange: ExchangeId::BybitFutures,
                message: format!("no funding history for {}", symbol),
                status: None,
            })
    }

    /// GET /v5/market/open-interest?category=linear&symbol={}&intervalTime=5min&limit=1
    pub async fn open_interest(&self, symbol: &Symbol) -> Result<BybitOpenInterestRaw> {
        let url = format!(
            "{}/v5/market/open-interest?category=linear&symbol={}&intervalTime=5min&limit=1",
            self.base_url,
            unified_to_bybit(symbol)
        );
        let result: BybitOpenInterestResult = self.fetch(&url).await?;
        result
            .list
            .into_iter()
            .next()
            .ok_or_else(|| GatewayError::Rest {
                exchange: ExchangeId::BybitFutures,
                message: format!("no open interest for {}", symbol),
                status: None,
            })
    }
}
