# Perpetual Futures Support — Design Document

**Date**: 2026-02-22
**Scope**: Market data only (no trading). Binance, Bybit, Bitget.

## Decisions

- **Scope**: Только market data (orderbook, trades, candles, tickers) + futures-специфичные данные
- **Crate layout**: Один крейт на биржу — spot и futures в подмодулях
- **API design**: Отдельные структуры BinanceSpot / BinanceFutures, обе реализуют Exchange trait
- **Approach**: Отдельный FuturesExchange trait для futures-специфичных данных
- **Data**: Полный набор — funding rate, mark price, open interest, ликвидации

## 1. gateway-core Changes

### 1.1 ExchangeId

```rust
pub enum ExchangeId {
    BinanceSpot,
    BinanceFutures,
    BybitSpot,
    BybitFutures,
    BitgetSpot,
    BitgetFutures,
    // reserved: Okx, Gate, Hyperliquid, Kucoin, Mexc
}
```

Старые варианты Binance/Bybit/Bitget переименовываются в *Spot. Breaking change допустим.

### 1.2 New Types

```rust
pub struct FundingRate {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub rate: Decimal,
    pub next_funding_time_ms: u64,
    pub timestamp_ms: u64,
}

pub struct MarkPrice {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub mark_price: Decimal,
    pub index_price: Decimal,
    pub timestamp_ms: u64,
}

pub struct OpenInterest {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub open_interest: Decimal,
    pub open_interest_value: Decimal,
    pub timestamp_ms: u64,
}

pub struct Liquidation {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Decimal,
    pub qty: Decimal,
    pub timestamp_ms: u64,
}
```

### 1.3 FuturesExchange Trait

```rust
#[async_trait]
pub trait FuturesExchange: Exchange {
    async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate>;
    async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice>;
    async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest>;
    async fn liquidations(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Liquidation>>;

    async fn stream_mark_price(&self, symbol: &Symbol) -> Result<BoxStream<MarkPrice>>;
    async fn stream_liquidations(&self, symbol: &Symbol) -> Result<BoxStream<Liquidation>>;
}
```

### 1.4 StreamEvent Extension

```rust
pub enum StreamEvent {
    OrderBook(OrderBook),
    Trade(Trade),
    Candle(Candle),
    Ticker(Ticker),
    FundingRate(FundingRate),
    MarkPrice(MarkPrice),
    Liquidation(Liquidation),
    Info(String),
}
```

## 2. Exchange Crates

### 2.1 File Structure (each exchange)

```
gateway-{exchange}/src/
├── lib.rs              # pub mod spot; pub mod futures; + re-exports
├── spot/
│   ├── mod.rs          # {Exchange}Spot + Exchange impl
│   ├── rest.rs         # Spot REST
│   ├── ws.rs           # Spot WebSocket
│   └── mapper.rs       # Spot mapping
└── futures/
    ├── mod.rs          # {Exchange}Futures + Exchange + FuturesExchange impl
    ├── rest.rs         # Futures REST
    ├── ws.rs           # Futures WebSocket
    └── mapper.rs       # Futures mapping
```

### 2.2 API Endpoints

**Binance Futures (USD-M)**:
- REST: `https://fapi.binance.com/fapi/v1/...`
- WS: `wss://fstream.binance.com/ws` or `/stream?streams=...`
- Key endpoints: depth, trades, klines, ticker/24hr, fundingRate, premiumIndex, openInterest, allForceOrders

**Bybit Futures (Linear)**:
- REST: `https://api.bybit.com/v5/market/...?category=linear`
- WS: `wss://stream.bybit.com/v5/public/linear`
- Same V5 endpoints, category=linear

**Bitget Futures (USDT-M)**:
- REST: `https://api.bitget.com/api/v2/mix/market/...`
- WS: `wss://ws.bitget.com/v2/ws/public`, instType=USDT-FUTURES
- `/mix/` path instead of `/spot/`

### 2.3 Renames

- `Binance` → `BinanceSpot` (type alias for compat)
- `Bybit` → `BybitSpot`
- `Bitget` → `BitgetSpot`

## 3. Gateway Manager

- BinanceSpot and BinanceFutures register as separate ExchangeId entries — works out of the box
- Add dual storage: `HashMap<ExchangeId, Arc<dyn FuturesExchange>>` alongside existing Exchange map
- New methods: `get_futures()`, `all_funding_rates()`, `stream_liquidations_multi()`

## 4. Testing

- Unit tests: mapper fixtures (raw JSON → unified types) for each exchange's futures API
- Integration examples in `examples/` for each futures gateway
- Verify existing spot code still compiles and works
