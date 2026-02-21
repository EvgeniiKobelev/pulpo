# pulpo_loco workspace architecture

## Структура

```
pulpo_loco/
├── Cargo.toml                    # workspace root
├── crates/
│   ├── gateway-core/             # типы, трейты, ошибки — 0 зависимостей от бирж
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs          # OrderBook, Trade, Candle, Ticker, Symbol...
│   │       ├── traits.rs         # Exchange, ExchangeStream
│   │       ├── error.rs          # GatewayError
│   │       ├── symbol.rs         # Symbol нормализация
│   │       ├── config.rs         # WsConfig, RestConfig, RateLimitConfig
│   │       └── stream.rs         # StreamEvent, Subscription handle
│   │
│   ├── gateway-binance/          # имплементация Binance
│   │   ├── Cargo.toml            # depends on gateway-core
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── rest.rs           # REST клиент
│   │       ├── ws.rs             # WebSocket клиент + reconnect
│   │       ├── mapper.rs         # JSON биржи → gateway-core типы
│   │       ├── symbols.rs        # BTC/USDT → "BTCUSDT"
│   │       └── rate_limit.rs
│   │
│   ├── gateway-bybit/
│   │   └── ...                   # аналогичная структура
│   │
│   ├── gateway-okx/
│   │   └── ...
│   │
│   └── gateway-manager/          # опционально: мультиплексор нескольких бирж
│       ├── Cargo.toml            # depends on gateway-core + все биржи через features
│       └── src/
│           ├── lib.rs
│           ├── multi.rs          # MultiExchange — агрегатор
│           └── registry.rs       # динамический реестр бирж
```

---

## gateway-core

### Cargo.toml

```toml
[package]
name = "gateway-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
tokio = { version = "1", features = ["sync"] }
tokio-stream = "0.1"
futures = "0.3"
rust_decimal = { version = "1", features = ["serde-with-str"] }
async-trait = "0.1"
```

### src/types.rs

```rust
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Идентификаторы ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExchangeId {
    Binance,
    Bybit,
    Okx,
    Gate,
    Hyperliquid,
    Kucoin,
    Mexc,
}

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binance => write!(f, "binance"),
            Self::Bybit => write!(f, "bybit"),
            Self::Okx => write!(f, "okx"),
            Self::Gate => write!(f, "gate"),
            Self::Hyperliquid => write!(f, "hyperliquid"),
            Self::Kucoin => write!(f, "kucoin"),
            Self::Mexc => write!(f, "mexc"),
        }
    }
}

// ── Символ (нормализованный) ──

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol {
    pub base: String,   // "BTC"
    pub quote: String,  // "USDT"
}

impl Symbol {
    pub fn new(base: impl Into<String>, quote: impl Into<String>) -> Self {
        Self {
            base: base.into().to_uppercase(),
            quote: quote.into().to_uppercase(),
        }
    }

    /// Удобный конструктор: Symbol::parse("BTC/USDT")
    pub fn parse(s: &str) -> Option<Self> {
        let (base, quote) = s.split_once('/')?;
        Some(Self::new(base.trim(), quote.trim()))
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.base, self.quote)
    }
}

// ── Рыночные данные ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Level {
    pub price: Decimal,
    pub qty: Decimal,
}

impl Level {
    pub fn new(price: Decimal, qty: Decimal) -> Self {
        Self { price, qty }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub timestamp_ms: u64,
    pub sequence: Option<u64>,
}

impl OrderBook {
    pub fn best_bid(&self) -> Option<&Level> {
        self.bids.first()
    }

    pub fn best_ask(&self) -> Option<&Level> {
        self.asks.first()
    }

    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_ask(), self.best_bid()) {
            (Some(ask), Some(bid)) => Some(ask.price - bid.price),
            _ => None,
        }
    }

    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_ask(), self.best_bid()) {
            (Some(ask), Some(bid)) => Some((ask.price + bid.price) / Decimal::TWO),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub price: Decimal,
    pub qty: Decimal,
    pub side: Side,
    pub timestamp_ms: u64,
    pub trade_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub open_time_ms: u64,
    pub close_time_ms: u64,
    pub is_closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticker {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub last_price: Decimal,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub volume_24h: Decimal,
    pub price_change_pct_24h: Option<Decimal>,
    pub timestamp_ms: u64,
}

// ── Интервал свечей ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interval {
    S1,
    M1,
    M3,
    M5,
    M15,
    M30,
    H1,
    H4,
    D1,
    W1,
}

impl Interval {
    pub fn as_secs(&self) -> u64 {
        match self {
            Self::S1 => 1,
            Self::M1 => 60,
            Self::M3 => 180,
            Self::M5 => 300,
            Self::M15 => 900,
            Self::M30 => 1800,
            Self::H1 => 3600,
            Self::H4 => 14400,
            Self::D1 => 86400,
            Self::W1 => 604800,
        }
    }
}

// ── Информация о бирже ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub symbol: Symbol,
    pub raw_symbol: String,          // как биржа называет: "BTCUSDT"
    pub status: SymbolStatus,
    pub base_precision: u8,
    pub quote_precision: u8,
    pub min_qty: Option<Decimal>,
    pub min_notional: Option<Decimal>,
    pub tick_size: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolStatus {
    Trading,
    Halted,
    PreTrading,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeInfo {
    pub exchange: ExchangeId,
    pub symbols: Vec<SymbolInfo>,
}
```

### src/stream.rs

```rust
use crate::types::*;
use std::pin::Pin;
use futures::Stream;

/// Единый ивент из WS стрима
#[derive(Debug, Clone)]
pub enum StreamEvent {
    OrderBook(OrderBook),
    Trade(Trade),
    Candle(Candle),
    Ticker(Ticker),
    /// Биржа отправила что-то неизвестное / системное
    Info(String),
}

/// Тип стрима — Pin<Box<dyn Stream>>
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

/// Handle для управления подпиской
pub struct Subscription {
    /// Drop = отписка
    _cancel: tokio::sync::oneshot::Sender<()>,
}

impl Subscription {
    pub fn new(cancel: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { _cancel: cancel }
    }
}
```

### src/config.rs

```rust
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct WsConfig {
    pub reconnect_delay: Duration,
    pub max_reconnect_attempts: Option<u32>,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    /// При пропуске sequence — пересинк стакана через REST
    pub orderbook_resync_on_gap: bool,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            reconnect_delay: Duration::from_secs(1),
            max_reconnect_attempts: None, // бесконечно
            ping_interval: Duration::from_secs(15),
            pong_timeout: Duration::from_secs(10),
            orderbook_resync_on_gap: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RestConfig {
    pub timeout: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_retries: 3,
            retry_delay: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExchangeConfig {
    pub rest: RestConfig,
    pub ws: WsConfig,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub passphrase: Option<String>,  // OKX и подобные
}

impl Default for ExchangeConfig {
    fn default() -> Self {
        Self {
            rest: RestConfig::default(),
            ws: WsConfig::default(),
            api_key: None,
            api_secret: None,
            passphrase: None,
        }
    }
}
```

### src/error.rs

```rust
use thiserror::Error;
use crate::types::ExchangeId;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("[{exchange}] REST error: {message}")]
    Rest {
        exchange: ExchangeId,
        message: String,
        status: Option<u16>,
    },

    #[error("[{exchange}] WebSocket error: {message}")]
    WebSocket {
        exchange: ExchangeId,
        message: String,
    },

    #[error("[{exchange}] Rate limited, retry after {retry_after_ms}ms")]
    RateLimited {
        exchange: ExchangeId,
        retry_after_ms: u64,
    },

    #[error("[{exchange}] Symbol not found: {symbol}")]
    SymbolNotFound {
        exchange: ExchangeId,
        symbol: String,
    },

    #[error("[{exchange}] Auth error: {message}")]
    Auth {
        exchange: ExchangeId,
        message: String,
    },

    #[error("[{exchange}] Parse error: {message}")]
    Parse {
        exchange: ExchangeId,
        message: String,
    },

    #[error("Connection lost to {exchange}")]
    Disconnected {
        exchange: ExchangeId,
    },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;
```

### src/traits.rs

```rust
use async_trait::async_trait;
use crate::{
    config::ExchangeConfig,
    error::Result,
    stream::BoxStream,
    types::*,
};

/// Основной трейт — каждая биржа реализует это
#[async_trait]
pub trait Exchange: Send + Sync + 'static {
    // ── Meta ──

    fn id(&self) -> ExchangeId;

    fn config(&self) -> &ExchangeConfig;

    // ── REST: Market Data ──

    /// Получить список всех торговых пар
    async fn exchange_info(&self) -> Result<ExchangeInfo>;

    /// Снапшот стакана
    async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook>;

    /// Последние сделки
    async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>>;

    /// Исторические свечи
    async fn candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
        limit: u16,
    ) -> Result<Vec<Candle>>;

    /// Тикер
    async fn ticker(&self, symbol: &Symbol) -> Result<Ticker>;

    /// Тикеры всех пар (для скринеров)
    async fn all_tickers(&self) -> Result<Vec<Ticker>>;

    // ── WebSocket Streams ──

    /// Стрим обновлений стакана (одна пара)
    async fn stream_orderbook(&self, symbol: &Symbol) -> Result<BoxStream<OrderBook>>;

    /// Стрим сделок (одна пара)
    async fn stream_trades(&self, symbol: &Symbol) -> Result<BoxStream<Trade>>;

    /// Стрим свечей
    async fn stream_candles(
        &self,
        symbol: &Symbol,
        interval: Interval,
    ) -> Result<BoxStream<Candle>>;

    // ── Batch WS (для скринеров — подписка на много пар сразу) ──

    /// Стрим стаканов для множества пар
    async fn stream_orderbooks_batch(
        &self,
        symbols: &[Symbol],
    ) -> Result<BoxStream<OrderBook>> {
        // дефолтная реализация — мерж отдельных стримов
        // биржи могут переопределить (binance combined stream)
        use futures::stream::SelectAll;
        let mut all = SelectAll::new();
        for sym in symbols {
            all.push(self.stream_orderbook(sym).await?);
        }
        Ok(Box::pin(all))
    }

    /// Стрим трейдов для множества пар
    async fn stream_trades_batch(
        &self,
        symbols: &[Symbol],
    ) -> Result<BoxStream<Trade>> {
        use futures::stream::SelectAll;
        let mut all = SelectAll::new();
        for sym in symbols {
            all.push(self.stream_trades(sym).await?);
        }
        Ok(Box::pin(all))
    }
}

/// Расширение для бирж с приватным API (ордера, балансы)
#[async_trait]
pub trait ExchangeTrading: Exchange {
    async fn balances(&self) -> Result<Vec<Balance>>;
    async fn place_order(&self, order: &NewOrder) -> Result<OrderResponse>;
    async fn cancel_order(&self, symbol: &Symbol, order_id: &str) -> Result<()>;
    async fn open_orders(&self, symbol: Option<&Symbol>) -> Result<Vec<Order>>;
}

// Типы для трейдинга (базовые)
#[derive(Debug, Clone)]
pub struct Balance {
    pub asset: String,
    pub free: rust_decimal::Decimal,
    pub locked: rust_decimal::Decimal,
}

#[derive(Debug, Clone)]
pub struct NewOrder {
    pub symbol: Symbol,
    pub side: Side,
    pub order_type: OrderType,
    pub qty: rust_decimal::Decimal,
    pub price: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, Copy)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone)]
pub struct OrderResponse {
    pub order_id: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub order_type: OrderType,
    pub price: rust_decimal::Decimal,
    pub qty: rust_decimal::Decimal,
    pub filled_qty: rust_decimal::Decimal,
}
```

### src/lib.rs

```rust
pub mod config;
pub mod error;
pub mod stream;
pub mod symbol;
pub mod traits;
pub mod types;

// Реэкспорт для удобства
pub use config::*;
pub use error::{GatewayError, Result};
pub use stream::*;
pub use traits::*;
pub use types::*;
```

---

## gateway-binance (пример имплементации)

### Cargo.toml

```toml
[package]
name = "gateway-binance"
version = "0.1.0"
edition = "2021"

[dependencies]
gateway-core = { path = "../gateway-core" }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
tokio-stream = "0.1"
futures = "0.3"
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rust_decimal = { version = "1", features = ["serde-with-str"] }
async-trait = "0.1"
tracing = "0.1"
url = "2"
```

### src/lib.rs

```rust
mod mapper;
mod rest;
mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct Binance {
    config: ExchangeConfig,
    rest: rest::BinanceRest,
}

impl Binance {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BinanceRest::new(&config);
        Self { config, rest }
    }

    /// Быстрый конструктор без ключей (только market data)
    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for Binance {
    fn id(&self) -> ExchangeId {
        ExchangeId::Binance
    }

    fn config(&self) -> &ExchangeConfig {
        &self.config
    }

    async fn exchange_info(&self) -> Result<ExchangeInfo> {
        self.rest.exchange_info().await
    }

    async fn orderbook(&self, symbol: &Symbol, depth: u16) -> Result<OrderBook> {
        self.rest.orderbook(symbol, depth).await
    }

    async fn trades(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Trade>> {
        self.rest.trades(symbol, limit).await
    }

    async fn candles(&self, symbol: &Symbol, interval: Interval, limit: u16) -> Result<Vec<Candle>> {
        self.rest.candles(symbol, interval, limit).await
    }

    async fn ticker(&self, symbol: &Symbol) -> Result<Ticker> {
        self.rest.ticker(symbol).await
    }

    async fn all_tickers(&self) -> Result<Vec<Ticker>> {
        self.rest.all_tickers().await
    }

    async fn stream_orderbook(&self, symbol: &Symbol) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbook(&self.config, symbol).await
    }

    async fn stream_trades(&self, symbol: &Symbol) -> Result<BoxStream<Trade>> {
        ws::stream_trades(&self.config, symbol).await
    }

    async fn stream_candles(&self, symbol: &Symbol, interval: Interval) -> Result<BoxStream<Candle>> {
        ws::stream_candles(&self.config, symbol, interval).await
    }

    // Binance поддерживает combined streams — переопределяем batch
    async fn stream_orderbooks_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbooks_combined(&self.config, symbols).await
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        ws::stream_trades_combined(&self.config, symbols).await
    }
}
```

### src/mapper.rs (JSON биржи → unified типы)

```rust
use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ── Binance raw JSON structs ──

#[derive(Deserialize)]
pub struct BinanceOrderBookRaw {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

#[derive(Deserialize)]
pub struct BinanceTradeRaw {
    pub id: u64,
    pub price: String,
    pub qty: String,
    pub time: u64,
    #[serde(rename = "isBuyerMaker")]
    pub is_buyer_maker: bool,
}

#[derive(Deserialize)]
pub struct BinanceWsDepthRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub bids: Vec<[String; 2]>,
    #[serde(rename = "a")]
    pub asks: Vec<[String; 2]>,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "u")]
    pub last_update_id: u64,
}

#[derive(Deserialize)]
pub struct BinanceWsTradeRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub qty: String,
    #[serde(rename = "T")]
    pub trade_time: u64,
    #[serde(rename = "t")]
    pub trade_id: u64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

// ── Маппинг в gateway-core типы ──

fn parse_levels(raw: &[[String; 2]]) -> Vec<Level> {
    raw.iter()
        .filter_map(|[p, q]| {
            Some(Level::new(
                Decimal::from_str(p).ok()?,
                Decimal::from_str(q).ok()?,
            ))
        })
        .collect()
}

impl BinanceOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::Binance,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: 0, // REST не даёт timestamp
            sequence: Some(self.last_update_id),
        }
    }
}

impl BinanceWsDepthRaw {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = binance_symbol_to_unified(&self.symbol);
        OrderBook {
            exchange: ExchangeId::Binance,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.event_time,
            sequence: Some(self.last_update_id),
        }
    }
}

impl BinanceWsTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Trade {
            exchange: ExchangeId::Binance,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker { Side::Sell } else { Side::Buy },
            timestamp_ms: self.trade_time,
            trade_id: Some(self.trade_id.to_string()),
        }
    }
}

// ── Symbol mapping ──

pub fn unified_to_binance(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote) // BTCUSDT
}

pub fn binance_symbol_to_unified(raw: &str) -> Symbol {
    // Простая эвристика — ищем известные quote assets
    for quote in &["USDT", "USDC", "BUSD", "BTC", "ETH", "BNB", "TUSD", "FDUSD"] {
        if let Some(base) = raw.strip_suffix(quote) {
            if !base.is_empty() {
                return Symbol::new(base, *quote);
            }
        }
    }
    // Fallback
    Symbol::new(raw, "UNKNOWN")
}

// ── Interval mapping ──

pub fn interval_to_binance(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1s",
        Interval::M1 => "1m",
        Interval::M3 => "3m",
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "1w",
    }
}
```

---

## gateway-manager (мультиплексор)

### Cargo.toml

```toml
[package]
name = "gateway-manager"
version = "0.1.0"
edition = "2021"

[dependencies]
gateway-core = { path = "../gateway-core" }
async-trait = "0.1"
futures = "0.3"
tokio = { version = "1", features = ["sync"] }

# Биржи через features
gateway-binance = { path = "../gateway-binance", optional = true }
gateway-bybit = { path = "../gateway-bybit", optional = true }
gateway-okx = { path = "../gateway-okx", optional = true }

[features]
default = ["binance"]
binance = ["dep:gateway-binance"]
bybit = ["dep:gateway-bybit"]
okx = ["dep:gateway-okx"]
all = ["binance", "bybit", "okx"]
```

### src/lib.rs

```rust
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

    /// Регистрация биржи
    pub fn register(&mut self, exchange: impl Exchange) -> &mut Self {
        let id = exchange.id();
        self.exchanges.insert(id, Arc::new(exchange));
        self
    }

    /// Получить биржу по ID
    pub fn get(&self, id: ExchangeId) -> Option<Arc<dyn Exchange>> {
        self.exchanges.get(&id).cloned()
    }

    /// Все зарегистрированные биржи
    pub fn all(&self) -> Vec<Arc<dyn Exchange>> {
        self.exchanges.values().cloned().collect()
    }

    /// Получить тикеры со всех бирж параллельно
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

    /// Стрим трейдов с нескольких бирж одновременно
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
```

---

## Пример использования в проекте

### Cargo.toml скринера

```toml
[package]
name = "my-screener"
version = "0.1.0"
edition = "2021"

[dependencies]
gateway-core = { path = "../pulpo_loco/crates/gateway-core" }
gateway-binance = { path = "../pulpo_loco/crates/gateway-binance" }
gateway-bybit = { path = "../pulpo_loco/crates/gateway-bybit" }
gateway-manager = { path = "../pulpo_loco/crates/gateway-manager", features = ["all"] }
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
futures = "0.3"
tracing = "0.1"
tracing-subscriber = "0.3"
```

### main.rs — простой скринер

```rust
use gateway_core::*;
use gateway_binance::Binance;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ── 1. Создаём подключение к бирже (одна строка!) ──
    let binance = Binance::public();

    // ── 2. REST: получить все пары ──
    let info = binance.exchange_info().await?;
    println!("Binance: {} торговых пар", info.symbols.len());

    // ── 3. REST: снапшот стакана ──
    let btc = Symbol::new("BTC", "USDT");
    let ob = binance.orderbook(&btc, 10).await?;
    println!(
        "BTC/USDT bid={} ask={} spread={}",
        ob.best_bid().unwrap().price,
        ob.best_ask().unwrap().price,
        ob.spread().unwrap()
    );

    // ── 4. WS: подписка на трейды ──
    let mut trades_stream = binance.stream_trades(&btc).await?;

    println!("\n--- Стрим трейдов BTC/USDT ---");
    while let Some(trade) = trades_stream.next().await {
        println!(
            "{} {:?} {} @ {} | {}",
            trade.symbol,
            trade.side,
            trade.qty,
            trade.price,
            trade.timestamp_ms
        );
    }

    Ok(())
}
```

### main.rs — мульти-биржевой скринер объёмов

```rust
use gateway_core::*;
use gateway_binance::Binance;
// use gateway_bybit::Bybit;
// use gateway_okx::Okx;
use gateway_manager::GatewayManager;
use tokio_stream::StreamExt;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ── 1. Регистрируем биржи ──
    let mut gw = GatewayManager::new();
    gw.register(Binance::public());
    // gw.register(Bybit::public());
    // gw.register(Okx::public());

    // ── 2. Сканируем тикеры со всех бирж ──
    let results = gw.all_tickers_everywhere().await;
    for (exchange, tickers) in &results {
        match tickers {
            Ok(t) => println!("{}: {} тикеров", exchange, t.len()),
            Err(e) => println!("{}: ошибка — {}", exchange, e),
        }
    }

    // ── 3. Находим топ-10 по объёму на Binance ──
    let binance = gw.get(ExchangeId::Binance).unwrap();
    let tickers = binance.all_tickers().await?;

    let mut usdt_tickers: Vec<_> = tickers
        .into_iter()
        .filter(|t| t.symbol.quote == "USDT")
        .collect();
    usdt_tickers.sort_by(|a, b| b.volume_24h.cmp(&a.volume_24h));

    let top10: Vec<Symbol> = usdt_tickers
        .iter()
        .take(10)
        .map(|t| t.symbol.clone())
        .collect();

    println!("\n--- Топ 10 по объёму ---");
    for (i, t) in usdt_tickers.iter().take(10).enumerate() {
        println!("{}. {} — vol: {}", i + 1, t.symbol, t.volume_24h);
    }

    // ── 4. Стрим трейдов по топ-10 (batch подписка) ──
    let mut stream = binance.stream_trades_batch(&top10).await?;

    // Простой volume tracker
    let volumes: Arc<Mutex<HashMap<String, Decimal>>> = Arc::new(Mutex::new(HashMap::new()));
    let volumes_clone = volumes.clone();

    // Печатаем аккумулированный объём каждые 5 секунд
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let v = volumes_clone.lock().await;
            if !v.is_empty() {
                println!("\n=== Volume last 5s ===");
                let mut sorted: Vec<_> = v.iter().collect();
                sorted.sort_by(|a, b| b.1.cmp(a.1));
                for (sym, vol) in sorted.iter().take(5) {
                    println!("  {} — ${}", sym, vol);
                }
            }
            drop(v);
            volumes_clone.lock().await.clear();
        }
    });

    while let Some(trade) = stream.next().await {
        let notional = trade.price * trade.qty;
        let mut v = volumes.lock().await;
        *v.entry(trade.symbol.to_string()).or_insert(Decimal::ZERO) += notional;
    }

    Ok(())
}
```

### main.rs — арбитражный бот (мульти-биржа)

```rust
use gateway_core::*;
use gateway_binance::Binance;
// use gateway_bybit::Bybit;
use gateway_manager::GatewayManager;
use tokio_stream::StreamExt;
use futures::stream::SelectAll;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let mut gw = GatewayManager::new();
    gw.register(Binance::public());
    // gw.register(Bybit::public());

    let btc = Symbol::new("BTC", "USDT");

    // Подписываемся на стаканы с двух бирж
    let pairs = vec![
        (ExchangeId::Binance, btc.clone()),
        // (ExchangeId::Bybit, btc.clone()),
    ];

    // Через manager — мультиплексированный стрим
    // В реальном арб-боте скорее всего будешь держать
    // последний стакан каждой биржи в DashMap и сравнивать
    let binance = gw.get(ExchangeId::Binance).unwrap();
    let mut ob_stream = binance.stream_orderbook(&btc).await?;

    println!("--- Watching BTC/USDT orderbook ---");
    while let Some(ob) = ob_stream.next().await {
        if let (Some(bid), Some(ask)) = (ob.best_bid(), ob.best_ask()) {
            println!(
                "[{}] bid={} ask={} spread={}",
                ob.exchange,
                bid.price,
                ask.price,
                ob.spread().unwrap()
            );
        }
    }

    Ok(())
}
```
