use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.gateio.ws/api/v4";
const EXCHANGE: ExchangeId = ExchangeId::GateFutures;

pub struct GateFuturesRest {
    client: Client,
    base_url: String,
}

impl GateFuturesRest {
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

    /// GET /futures/usdt/contracts
    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let url = format!("{}/futures/usdt/contracts", self.base_url);
        let data: Vec<GateFuturesContractRaw> = self.fetch(&url).await?;
        Ok(contracts_to_exchange_info(data))
    }

    /// GET /futures/usdt/order_book?contract={}&limit={}&with_id=true
    pub async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        let contract = unified_to_gate(symbol);
        let depth = depth.min(50);
        let url = format!(
            "{}/futures/usdt/order_book?contract={}&limit={}&with_id=true",
            self.base_url, contract, depth
        );
        let raw: GateFuturesOrderBookRaw = self.fetch(&url).await?;
        Ok(raw.into_orderbook(symbol.clone()))
    }

    /// GET /futures/usdt/trades?contract={}&limit={}
    pub async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        let contract = unified_to_gate(symbol);
        let limit = limit.min(1000);
        let url = format!(
            "{}/futures/usdt/trades?contract={}&limit={}",
            self.base_url, contract, limit
        );
        let data: Vec<GateFuturesTradeRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|t| t.into_trade()).collect())
    }

    /// GET /futures/usdt/candlesticks?contract={}&interval={}&limit={}
    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let contract = unified_to_gate(symbol);
        let limit = limit.min(999);
        let url = format!(
            "{}/futures/usdt/candlesticks?contract={}&interval={}&limit={}",
            self.base_url,
            contract,
            interval_to_gate_futures(interval),
            limit
        );
        let data: Vec<GateFuturesCandleRaw> = self.fetch(&url).await?;
        let sym = symbol.clone();
        Ok(data
            .into_iter()
            .filter_map(|c| c.into_candle(sym.clone()))
            .collect())
    }

    /// GET /futures/usdt/tickers?contract={}
    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let contract = unified_to_gate(symbol);
        let url = format!(
            "{}/futures/usdt/tickers?contract={}",
            self.base_url, contract
        );
        let data: Vec<GateFuturesTickerRaw> = self.fetch(&url).await?;
        data.into_iter()
            .next()
            .map(|t| t.into_ticker())
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: EXCHANGE,
                symbol: symbol.to_string(),
            })
    }

    /// GET /futures/usdt/tickers
    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let url = format!("{}/futures/usdt/tickers", self.base_url);
        let data: Vec<GateFuturesTickerRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|t| t.into_ticker()).collect())
    }

    /// GET /futures/usdt/tickers?contract={} — returns raw ticker for futures-specific data.
    pub async fn ticker_raw(&self, symbol: &Symbol) -> Result<GateFuturesTickerRaw> {
        let contract = unified_to_gate(symbol);
        let url = format!(
            "{}/futures/usdt/tickers?contract={}",
            self.base_url, contract
        );
        let data: Vec<GateFuturesTickerRaw> = self.fetch(&url).await?;
        data.into_iter()
            .next()
            .ok_or_else(|| GatewayError::SymbolNotFound {
                exchange: EXCHANGE,
                symbol: symbol.to_string(),
            })
    }

    /// GET /futures/usdt/liq_orders?contract={}&limit={}
    pub async fn liq_orders(
        &self,
        symbol: &Symbol,
        limit: u16,
    ) -> Result<Vec<Liquidation>> {
        let contract = unified_to_gate(symbol);
        let url = format!(
            "{}/futures/usdt/liq_orders?contract={}&limit={}",
            self.base_url, contract, limit
        );
        let data: Vec<GateFuturesLiqOrderRaw> = self.fetch(&url).await?;
        Ok(data.into_iter().map(|l| l.into_liquidation()).collect())
    }
}
