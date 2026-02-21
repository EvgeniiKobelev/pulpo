use gateway_core::*;
use std::collections::HashMap;
use std::sync::Arc;

pub struct GatewayManager {
    exchanges: HashMap<ExchangeId, Arc<dyn Exchange>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self {
            exchanges: HashMap::new(),
        }
    }

    /// Register an exchange
    pub fn register(&mut self, exchange: impl Exchange) -> &mut Self {
        let id = exchange.id();
        self.exchanges.insert(id, Arc::new(exchange));
        self
    }

    /// Get exchange by ID
    pub fn get(&self, id: ExchangeId) -> Option<Arc<dyn Exchange>> {
        self.exchanges.get(&id).cloned()
    }

    /// All registered exchanges
    pub fn all(&self) -> Vec<Arc<dyn Exchange>> {
        self.exchanges.values().cloned().collect()
    }

    /// Get tickers from all exchanges in parallel
    pub async fn all_tickers_everywhere(&self) -> Vec<(ExchangeId, Result<Vec<Ticker>>)> {
        let mut handles = vec![];
        for (id, ex) in &self.exchanges {
            let ex = ex.clone();
            let id = *id;
            handles.push(tokio::spawn(async move {
                (id, ex.all_tickers().await)
            }));
        }
        let mut results = vec![];
        for h in handles {
            if let Ok(r) = h.await {
                results.push(r);
            }
        }
        results
    }

    /// Stream trades from multiple exchanges
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
}

impl Default for GatewayManager {
    fn default() -> Self {
        Self::new()
    }
}
