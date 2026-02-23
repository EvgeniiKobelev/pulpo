use crate::spot::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.gateio.ws/api/v4";
const EXCHANGE: ExchangeId = ExchangeId::Gate;

pub struct GateRest {
    client: Client,
    base_url: String,
}

impl GateRest {
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

    /// Helper: send GET, check HTTP status, deserialize JSON.
    ///
    /// Gate.io REST responses have no wrapper — data is returned directly.
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

        resp.json::<T>().await.map_err(|e| GatewayError::Parse {
            exchange: EXCHANGE,
            message: e.to_string(),
        })
    }

    /// GET /spot/currency_pairs
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/spot/currency_pairs", self.base_url);
        let data: Vec<GateSymbolRaw> = self.fetch(&url).await?;
        Ok(symbols_to_exchange_info(data))
    }

    /// GET /spot/order_book?currency_pair={}&limit={}
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let pair = unified_to_gate(symbol);
        let depth = depth.min(50);
        let url = format!(
            "{}/spot/order_book?currency_pair={}&limit={}&with_id=true",
            self.base_url, pair, depth
        );
        let raw: GateOrderBookRaw = self.fetch(&url).await?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /spot/trades?currency_pair={}&limit={}
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let pair = unified_to_gate(symbol);
        let limit = limit.min(1000);
        let url = format!(
            "{}/spot/trades?currency_pair={}&limit={}",
            self.base_url, pair, limit
        );
        let data: Vec<GateTradeRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|t| t.into_trade()).collect())
    }

    /// GET /spot/candlesticks?currency_pair={}&interval={}&limit={}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let pair = unified_to_gate(symbol);
        let limit = limit.min(999);
        let url = format!(
            "{}/spot/candlesticks?currency_pair={}&interval={}&limit={}",
            self.base_url,
            pair,
            interval_to_gate_rest(interval),
            limit
        );
        let data: Vec<Vec<String>> = self.fetch(&url).await?;
        let sym = symbol.clone();
        let candles: Vec<Candle> = data
            .iter()
            .filter_map(|row| parse_kline_row(row, sym.clone()))
            .collect();
        Ok(candles)
    }

    /// GET /spot/tickers?currency_pair={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let pair = unified_to_gate(symbol);
        let url = format!(
            "{}/spot/tickers?currency_pair={}",
            self.base_url, pair
        );
        let data: Vec<GateTickerRaw> = self.fetch(&url).await?;
        data.into_iter()
            .next()
            .map(|t| t.into_ticker())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: EXCHANGE,
                symbol: symbol.to_string(),
            })
    }

    /// GET /spot/tickers
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/spot/tickers", self.base_url);
        let data: Vec<GateTickerRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|t| t.into_ticker()).collect())
    }
}
