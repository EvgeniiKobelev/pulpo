use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api-futures.kucoin.com";
const EXCHANGE: ExchangeId = ExchangeId::KucoinFutures;

pub struct KucoinFuturesRest {
    client: Client,
    base_url: String,
}

impl KucoinFuturesRest {
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

    /// Helper: send GET, check HTTP status, deserialize KuCoin response wrapper.
    ///
    /// KuCoin wraps all responses in `{ "code": "200000", "data": ... }`.
    /// Error responses may omit `data` entirely, so we parse as Value first.
    async fn fetch<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
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

        let json: serde_json::Value =
            resp.json().await.map_err(|e| GatewayError::Parse {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;

        let code = json
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if code != "200000" {
            let msg = json
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("API error code={code}: {msg}"),
                status: None,
            });
        }

        let data = json
            .get("data")
            .cloned()
            .ok_or_else(|| GatewayError::Parse {
                exchange: EXCHANGE,
                message: "missing 'data' field in response".to_string(),
            })?;

        serde_json::from_value(data).map_err(|e| GatewayError::Parse {
            exchange: EXCHANGE,
            message: e.to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    /// GET /api/v1/contracts/active
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/api/v1/contracts/active", self.base_url);
        let data: Vec<KfContractRaw> = self.fetch(&url).await?;
        let symbols = data
            .into_iter()
            .map(|c| c.into_symbol_info())
            .collect();

        Ok(ExchangeInfo {
            exchange: EXCHANGE,
            symbols,
        })
    }

    /// GET /api/v1/level2/depth{20,100}?symbol=...
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let kf_sym = unified_to_kucoin_futures(symbol);
        let level = if depth <= 20 { 20 } else { 100 };
        let url = format!(
            "{}/api/v1/level2/depth{}?symbol={}",
            self.base_url, level, kf_sym
        );
        let raw: KfOrderBookRaw = self.fetch(&url).await?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /api/v1/trade/history?symbol=...
    pub async fn trades(&self, symbol: &Symbol, _limit: u16) -> Result<Vec<Trade>> {
        let kf_sym = unified_to_kucoin_futures(symbol);
        let url = format!(
            "{}/api/v1/trade/history?symbol={}",
            self.base_url, kf_sym
        );
        let data: Vec<KfTradeRaw> = self.fetch(&url).await?;
        let sym = symbol.clone();
        Ok(data.into_iter().map(|t| t.into_trade(sym.clone())).collect())
    }

    /// GET /api/v1/kline/query?symbol=...&granularity=...&from=...&to=...
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let kf_sym = unified_to_kucoin_futures(symbol);
        let granularity = interval_to_granularity(interval);
        let now = now_ms();
        let interval_ms = interval.as_secs() * 1000;
        let from = now.saturating_sub(interval_ms * limit as u64);
        let url = format!(
            "{}/api/v1/kline/query?symbol={}&granularity={}&from={}&to={}",
            self.base_url, kf_sym, granularity, from, now
        );
        let data: Vec<Vec<serde_json::Value>> = self.fetch(&url).await?;
        let sym = symbol.clone();
        let candles = data
            .iter()
            .take(limit as usize)
            .filter_map(|row| parse_kline_row(row, sym.clone(), interval))
            .collect();
        Ok(candles)
    }

    /// GET /api/v1/ticker?symbol=... + GET /api/v1/24hr-stats?symbol=...
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let kf_sym = unified_to_kucoin_futures(symbol);

        // Get 24hr stats for volume/change
        let stats_url = format!(
            "{}/api/v1/ticker?symbol={}",
            self.base_url, kf_sym
        );
        let ticker_raw: KfTickerRaw = self.fetch(&stats_url).await?;

        let ts = ticker_raw.ts.map(ns_to_ms).unwrap_or_else(now_ms);

        Ok(Ticker {
            exchange: EXCHANGE,
            symbol: symbol.clone(),
            last_price: ticker_raw
                .price
                .as_deref()
                .and_then(|s| rust_decimal::Decimal::from_str_exact(s).ok())
                .unwrap_or_default(),
            bid: ticker_raw
                .best_bid_price
                .as_deref()
                .and_then(|s| rust_decimal::Decimal::from_str_exact(s).ok()),
            ask: ticker_raw
                .best_ask_price
                .as_deref()
                .and_then(|s| rust_decimal::Decimal::from_str_exact(s).ok()),
            volume_24h: rust_decimal::Decimal::ZERO,
            price_change_pct_24h: None,
            timestamp_ms: ts,
        })
    }

    /// GET /api/v1/contracts/active — used for all_tickers with volume from contracts
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/api/v1/contracts/active", self.base_url);
        let data: Vec<KfContractRaw> = self.fetch(&url).await?;
        Ok(data
            .into_iter()
            .filter(|c| c.status.as_deref() == Some("Open"))
            .map(|c| c.into_ticker())
            .collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    /// GET /api/v1/funding-rate/{symbol}/current
    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let kf_sym = unified_to_kucoin_futures(symbol);
        let url = format!(
            "{}/api/v1/funding-rate/{}/current",
            self.base_url, kf_sym
        );
        let raw: KfFundingRateRaw = self.fetch(&url).await?;
        Ok(raw.into_funding_rate(symbol))
    }

    /// GET /api/v1/mark-price/{symbol}/current
    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let kf_sym = unified_to_kucoin_futures(symbol);
        let url = format!(
            "{}/api/v1/mark-price/{}/current",
            self.base_url, kf_sym
        );
        let raw: KfMarkPriceRaw = self.fetch(&url).await?;
        Ok(raw.into_mark_price(symbol))
    }

    /// GET /api/v1/contracts/{symbol} — open interest from contract detail
    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let kf_sym = unified_to_kucoin_futures(symbol);
        let url = format!("{}/api/v1/contracts/{}", self.base_url, kf_sym);
        let raw: KfContractRaw = self.fetch(&url).await?;

        let oi = raw
            .open_interest
            .as_deref()
            .and_then(|s| rust_decimal::Decimal::from_str_exact(s).ok())
            .unwrap_or_default();

        let oi_value = raw
            .last_trade_price
            .and_then(|p| rust_decimal::Decimal::try_from(p).ok())
            .map(|price| oi * price)
            .unwrap_or_default();

        Ok(OpenInterest {
            exchange: EXCHANGE,
            symbol: symbol.clone(),
            open_interest: oi,
            open_interest_value: oi_value,
            timestamp_ms: now_ms(),
        })
    }

    /// POST /api/v1/bullet-public — get WS connection token and endpoint.
    pub async fn bullet_public(&self) -> Result<KfBulletResponse> {
        let url = format!("{}/api/v1/bullet-public", self.base_url);
        let resp = self.client.post(&url).send().await.map_err(|e| {
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
                message: format!("bullet-public HTTP {status}: {body}"),
                status: Some(status),
            });
        }

        let wrapper: KucoinFuturesResponse<KfBulletResponse> =
            resp.json().await.map_err(|e| GatewayError::Parse {
                exchange: EXCHANGE,
                message: e.to_string(),
            })?;

        if wrapper.code != "200000" {
            return Err(GatewayError::Rest {
                exchange: EXCHANGE,
                message: format!("bullet-public API error: {}", wrapper.code),
                status: None,
            });
        }

        Ok(wrapper.data)
    }
}

// ---------------------------------------------------------------------------
// Kline row parser
// ---------------------------------------------------------------------------

/// Parse a KuCoin Futures kline row.
/// Row format: [time_ms, open, high, low, close, volume]
fn parse_kline_row(
    row: &[serde_json::Value],
    symbol: Symbol,
    interval: Interval,
) -> Option<Candle> {
    if row.len() < 6 {
        return None;
    }

    let open_time_ms = row[0].as_u64().or_else(|| {
        row[0].as_f64().map(|f| f as u64)
    })?;

    let open = parse_value_decimal(Some(&row[1]));
    let high = parse_value_decimal(Some(&row[2]));
    let low = parse_value_decimal(Some(&row[3]));
    let close = parse_value_decimal(Some(&row[4]));
    let volume = parse_value_decimal(Some(&row[5]));

    let interval_ms = interval.as_secs() * 1000;
    let close_time_ms = open_time_ms + interval_ms;

    Some(Candle {
        exchange: EXCHANGE,
        symbol,
        open,
        high,
        low,
        close,
        volume,
        open_time_ms,
        close_time_ms,
        is_closed: true,
    })
}
