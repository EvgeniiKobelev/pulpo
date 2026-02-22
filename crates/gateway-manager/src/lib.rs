use gateway_core::*;
use std::collections::HashMap;
use std::sync::Arc;

pub struct GatewayManager {
    exchanges: HashMap<ExchangeId, Arc<dyn Exchange>>,
    futures_exchanges: HashMap<ExchangeId, Arc<dyn FuturesExchange>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self {
            exchanges: HashMap::new(),
            futures_exchanges: HashMap::new(),
        }
    }

    /// Register a spot or generic exchange.
    pub fn register(&mut self, exchange: impl Exchange) -> &mut Self {
        let id = exchange.id();
        self.exchanges.insert(id, Arc::new(exchange));
        self
    }

    /// Register a futures exchange (also available as a regular Exchange).
    pub fn register_futures<T: FuturesExchange>(&mut self, exchange: T) -> &mut Self {
        let id = exchange.id();
        let arc = Arc::new(exchange);
        self.exchanges.insert(id, arc.clone() as Arc<dyn Exchange>);
        self.futures_exchanges
            .insert(id, arc as Arc<dyn FuturesExchange>);
        self
    }

    /// Get exchange by ID.
    pub fn get(&self, id: ExchangeId) -> Option<Arc<dyn Exchange>> {
        self.exchanges.get(&id).cloned()
    }

    /// Get futures exchange by ID.
    pub fn get_futures(&self, id: ExchangeId) -> Option<Arc<dyn FuturesExchange>> {
        self.futures_exchanges.get(&id).cloned()
    }

    /// All registered exchanges.
    pub fn all(&self) -> Vec<Arc<dyn Exchange>> {
        self.exchanges.values().cloned().collect()
    }

    /// Get tickers from all exchanges in parallel.
    pub async fn all_tickers_everywhere(&self) -> Vec<(ExchangeId, Result<Vec<Ticker>>)> {
        let mut handles = vec![];
        for (id, ex) in &self.exchanges {
            let ex = ex.clone();
            let id = *id;
            handles.push(tokio::spawn(async move { (id, ex.all_tickers().await) }));
        }
        let mut results = vec![];
        for h in handles {
            if let Ok(r) = h.await {
                results.push(r);
            }
        }
        results
    }

    /// Stream trades from multiple exchanges.
    pub async fn stream_trades_multi(
        &self,
        pairs: &[(ExchangeId, Symbol)],
    ) -> Result<BoxStream<Trade>> {
        use futures::stream::SelectAll;
        let mut all = SelectAll::new();
        for (exchange_id, symbol) in pairs {
            let ex = self.get(*exchange_id).ok_or_else(|| {
                GatewayError::Other(format!("Exchange {} not registered", exchange_id))
            })?;
            all.push(ex.stream_trades(symbol).await?);
        }
        Ok(Box::pin(all))
    }

    /// Get funding rates from all futures exchanges in parallel.
    pub async fn all_funding_rates(
        &self,
        symbol: &Symbol,
    ) -> Vec<(ExchangeId, Result<FundingRate>)> {
        let mut handles = vec![];
        for (id, ex) in &self.futures_exchanges {
            let ex = ex.clone();
            let id = *id;
            let sym = symbol.clone();
            handles.push(tokio::spawn(
                async move { (id, ex.funding_rate(&sym).await) },
            ));
        }
        let mut results = vec![];
        for h in handles {
            if let Ok(r) = h.await {
                results.push(r);
            }
        }
        results
    }

    /// Stream liquidations from multiple futures exchanges.
    pub async fn stream_liquidations_multi(
        &self,
        pairs: &[(ExchangeId, Symbol)],
    ) -> Result<BoxStream<Liquidation>> {
        use futures::stream::SelectAll;
        let mut all = SelectAll::new();
        for (exchange_id, symbol) in pairs {
            let ex = self.get_futures(*exchange_id).ok_or_else(|| {
                GatewayError::Other(format!("Futures exchange {} not registered", exchange_id))
            })?;
            all.push(ex.stream_liquidations(symbol).await?);
        }
        Ok(Box::pin(all))
    }
}

impl Default for GatewayManager {
    fn default() -> Self {
        Self::new()
    }
}
