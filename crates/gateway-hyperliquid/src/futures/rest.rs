use crate::futures::mapper::*;
use gateway_core::*;
use reqwest::Client;

const BASE_URL: &str = "https://api.hyperliquid.xyz";

pub struct HyperliquidFuturesRest {
    client: Client,
    base_url: String,
}

impl HyperliquidFuturesRest {
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

    /// Helper: POST /info with a JSON body.
    async fn post_info(&self, body: serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}/info", self.base_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| {
            GatewayError::Rest {
                exchange: ExchangeId::Hyperliquid,
                message: e.to_string(),
                status: None,
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Rest {
                exchange: ExchangeId::Hyperliquid,
                message: text,
                status: Some(status),
            });
        }

        resp.json::<serde_json::Value>().await.map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::Hyperliquid,
                message: e.to_string(),
            }
        })
    }

    /// `{"type": "meta"}` → HlMetaRaw
    pub async fn meta(&self) -> Result<HlMetaRaw> {
        let val = self.post_info(serde_json::json!({"type": "meta"})).await?;
        serde_json::from_value(val).map_err(|e| GatewayError::Parse {
            exchange: ExchangeId::Hyperliquid,
            message: e.to_string(),
        })
    }

    /// `{"type": "metaAndAssetCtxs"}` → (HlMetaRaw, Vec<HlAssetCtxRaw>)
    ///
    /// The response is a JSON array: `[meta, [ctx0, ctx1, ...]]`.
    pub async fn meta_and_asset_ctxs(&self) -> Result<(HlMetaRaw, Vec<HlAssetCtxRaw>)> {
        let val = self
            .post_info(serde_json::json!({"type": "metaAndAssetCtxs"}))
            .await?;
        let arr = val.as_array().ok_or_else(|| GatewayError::Parse {
            exchange: ExchangeId::Hyperliquid,
            message: "expected array from metaAndAssetCtxs".to_string(),
        })?;
        if arr.len() < 2 {
            return Err(GatewayError::Parse {
                exchange: ExchangeId::Hyperliquid,
                message: "metaAndAssetCtxs array too short".to_string(),
            });
        }
        let meta: HlMetaRaw =
            serde_json::from_value(arr[0].clone()).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::Hyperliquid,
                message: format!("meta parse: {e}"),
            })?;
        let ctxs: Vec<HlAssetCtxRaw> =
            serde_json::from_value(arr[1].clone()).map_err(|e| GatewayError::Parse {
                exchange: ExchangeId::Hyperliquid,
                message: format!("assetCtxs parse: {e}"),
            })?;
        Ok((meta, ctxs))
    }

    /// `{"type": "allMids"}` → HashMap of coin → mid price string
    #[allow(dead_code)]
    pub async fn all_mids(&self) -> Result<std::collections::HashMap<String, String>> {
        let val = self
            .post_info(serde_json::json!({"type": "allMids"}))
            .await?;
        serde_json::from_value(val).map_err(|e| GatewayError::Parse {
            exchange: ExchangeId::Hyperliquid,
            message: e.to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Exchange trait helpers
    // -----------------------------------------------------------------------

    pub async fn exchange_info(&self) -> Result<ExchangeInfo> {
        let meta = self.meta().await?;
        Ok(meta.into_exchange_info())
    }

    pub async fn orderbook(&self, symbol: &Symbol, _depth: u16) -> Result<OrderBook> {
        let val = self
            .post_info(serde_json::json!({
                "type": "l2Book",
                "coin": unified_to_hl(symbol)
            }))
            .await?;
        let raw: HlL2BookRaw = serde_json::from_value(val).map_err(|e| GatewayError::Parse {
            exchange: ExchangeId::Hyperliquid,
            message: e.to_string(),
        })?;
        Ok(raw.into_orderbook())
    }

    pub async fn trades(&self, _symbol: &Symbol, _limit: u16) -> Result<Vec<Trade>> {
        Err(GatewayError::Rest {
            exchange: ExchangeId::Hyperliquid,
            message: "Hyperliquid does not provide a public REST trades endpoint; use WebSocket stream_trades instead".to_string(),
            status: None,
        })
    }

    pub async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let interval_ms = interval.as_secs() * 1000;
        let start_time = now_ms.saturating_sub(interval_ms * limit as u64);

        let val = self
            .post_info(serde_json::json!({
                "type": "candleSnapshot",
                "req": {
                    "coin": unified_to_hl(symbol),
                    "interval": interval_to_hl(interval),
                    "startTime": start_time,
                    "endTime": now_ms
                }
            }))
            .await?;
        let raw: Vec<HlCandleRaw> = serde_json::from_value(val).map_err(|e| {
            GatewayError::Parse {
                exchange: ExchangeId::Hyperliquid,
                message: e.to_string(),
            }
        })?;
        Ok(raw.into_iter().map(|c| c.into_candle()).collect())
    }

    pub async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        let (meta, ctxs) = self.meta_and_asset_ctxs().await?;
        let coin = unified_to_hl(symbol);
        for (asset, ctx) in meta.universe.iter().zip(ctxs.into_iter()) {
            if asset.name == coin {
                return Ok(ctx.into_ticker(&coin));
            }
        }
        Err(GatewayError::SymbolNotFound {
            exchange: ExchangeId::Hyperliquid,
            symbol: symbol.to_string(),
        })
    }

    pub async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        let (meta, ctxs) = self.meta_and_asset_ctxs().await?;
        Ok(meta
            .universe
            .iter()
            .zip(ctxs.into_iter())
            .map(|(asset, ctx)| ctx.into_ticker(&asset.name))
            .collect())
    }

    // -----------------------------------------------------------------------
    // FuturesExchange trait helpers
    // -----------------------------------------------------------------------

    /// Find asset context for a given coin from metaAndAssetCtxs.
    async fn find_asset_ctx(&self, symbol: &Symbol) -> Result<(String, HlAssetCtxRaw)> {
        let (meta, ctxs) = self.meta_and_asset_ctxs().await?;
        let coin = unified_to_hl(symbol);
        for (asset, ctx) in meta.universe.into_iter().zip(ctxs.into_iter()) {
            if asset.name == coin {
                return Ok((coin, ctx));
            }
        }
        Err(GatewayError::SymbolNotFound {
            exchange: ExchangeId::Hyperliquid,
            symbol: symbol.to_string(),
        })
    }

    pub async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let (coin, ctx) = self.find_asset_ctx(symbol).await?;
        Ok(ctx.into_funding_rate(&coin))
    }

    pub async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let (coin, ctx) = self.find_asset_ctx(symbol).await?;
        Ok(ctx.into_mark_price(&coin))
    }

    pub async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let (coin, ctx) = self.find_asset_ctx(symbol).await?;
        Ok(ctx.into_open_interest(&coin))
    }
}
