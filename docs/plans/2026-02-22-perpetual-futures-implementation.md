# Perpetual Futures Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add perpetual futures market data support (orderbook, trades, candles, tickers, funding rate, mark price, open interest, liquidations) for Binance, Bybit, and Bitget exchanges.

**Architecture:** Each exchange crate (gateway-binance, gateway-bybit, gateway-bitget) gets refactored to have `spot/` and `futures/` submodules. A new `FuturesExchange` trait in gateway-core provides futures-specific methods. `ExchangeId` enum gets split into `BinanceSpot`/`BinanceFutures`/etc.

**Tech Stack:** Rust, tokio, reqwest, tokio-tungstenite, serde, rust_decimal, async-trait, futures

---

## Task 1: gateway-core — Update ExchangeId enum

**Files:**
- Modify: `crates/gateway-core/src/types.rs:5-31`

**Step 1: Update ExchangeId enum and Display impl**

Replace `ExchangeId` enum at `types.rs:5-31`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeId {
    BinanceSpot,
    BinanceFutures,
    BitgetSpot,
    BitgetFutures,
    BybitSpot,
    BybitFutures,
    Okx,
    Gate,
    Hyperliquid,
    Kucoin,
    Mexc,
}

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BinanceSpot => write!(f, "binance_spot"),
            Self::BinanceFutures => write!(f, "binance_futures"),
            Self::BitgetSpot => write!(f, "bitget_spot"),
            Self::BitgetFutures => write!(f, "bitget_futures"),
            Self::BybitSpot => write!(f, "bybit_spot"),
            Self::BybitFutures => write!(f, "bybit_futures"),
            Self::Okx => write!(f, "okx"),
            Self::Gate => write!(f, "gate"),
            Self::Hyperliquid => write!(f, "hyperliquid"),
            Self::Kucoin => write!(f, "kucoin"),
            Self::Mexc => write!(f, "mexc"),
        }
    }
}
```

**Step 2: Verify compile**

Run: `cargo check -p gateway-core`
Expected: PASS (core itself compiles, downstream crates will break — that's expected)

**Step 3: Commit**

```bash
git add crates/gateway-core/src/types.rs
git commit -m "refactor: split ExchangeId into Spot/Futures variants"
```

---

## Task 2: gateway-core — Add futures types

**Files:**
- Modify: `crates/gateway-core/src/types.rs` (append after Interval at line ~151)

**Step 1: Add FundingRate, MarkPrice, OpenInterest, Liquidation types**

Append after the `Interval` impl block (after line 150):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRate {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub rate: Decimal,
    pub next_funding_time_ms: u64,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkPrice {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub mark_price: Decimal,
    pub index_price: Decimal,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenInterest {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub open_interest: Decimal,
    pub open_interest_value: Decimal,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Liquidation {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Decimal,
    pub qty: Decimal,
    pub timestamp_ms: u64,
}
```

**Step 2: Update StreamEvent in `crates/gateway-core/src/stream.rs:6-12`**

```rust
#[derive(Debug, Clone)]
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

**Step 3: Verify compile**

Run: `cargo check -p gateway-core`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/gateway-core/src/types.rs crates/gateway-core/src/stream.rs
git commit -m "feat: add FundingRate, MarkPrice, OpenInterest, Liquidation types"
```

---

## Task 3: gateway-core — Add FuturesExchange trait

**Files:**
- Modify: `crates/gateway-core/src/traits.rs` (append after ExchangeTrading)
- Modify: `crates/gateway-core/src/lib.rs` (re-exports already cover via `pub use traits::*`)

**Step 1: Add FuturesExchange trait**

Append after `Order` struct (after line 86 in `traits.rs`):

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

**Step 2: Verify compile**

Run: `cargo check -p gateway-core`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/gateway-core/src/traits.rs
git commit -m "feat: add FuturesExchange trait"
```

---

## Task 4: gateway-binance — Refactor spot into submodule

This task moves existing Binance code into a `spot/` submodule and renames `Binance` to `BinanceSpot`.

**Files:**
- Modify: `crates/gateway-binance/src/lib.rs` (rewrite)
- Create: `crates/gateway-binance/src/spot/mod.rs` (from old lib.rs)
- Move: `crates/gateway-binance/src/mapper.rs` → `crates/gateway-binance/src/spot/mapper.rs`
- Move: `crates/gateway-binance/src/rest.rs` → `crates/gateway-binance/src/spot/rest.rs`
- Move: `crates/gateway-binance/src/ws.rs` → `crates/gateway-binance/src/spot/ws.rs`

**Step 1: Create spot/ directory and move files**

```bash
mkdir -p crates/gateway-binance/src/spot
mv crates/gateway-binance/src/mapper.rs crates/gateway-binance/src/spot/mapper.rs
mv crates/gateway-binance/src/rest.rs crates/gateway-binance/src/spot/rest.rs
mv crates/gateway-binance/src/ws.rs crates/gateway-binance/src/spot/ws.rs
```

**Step 2: Create `spot/mod.rs`**

Write `crates/gateway-binance/src/spot/mod.rs`:

```rust
pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct BinanceSpot {
    config: ExchangeConfig,
    rest: rest::BinanceRest,
}

impl BinanceSpot {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BinanceRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for BinanceSpot {
    fn id(&self) -> ExchangeId {
        ExchangeId::BinanceSpot
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

    async fn stream_orderbooks_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbooks_combined(&self.config, symbols).await
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        ws::stream_trades_combined(&self.config, symbols).await
    }
}
```

**Step 3: Update all `ExchangeId::Binance` references in spot submodule**

In `spot/mapper.rs`: Replace all `ExchangeId::Binance` with `ExchangeId::BinanceSpot` (there are ~10 occurrences in the file and tests).

In `spot/rest.rs`: Replace all `ExchangeId::Binance` with `ExchangeId::BinanceSpot`.

In `spot/ws.rs`: Replace all `ExchangeId::Binance` with `ExchangeId::BinanceSpot`.

**Step 4: Rewrite `lib.rs`**

Write `crates/gateway-binance/src/lib.rs`:

```rust
pub mod spot;
pub mod futures;

pub use spot::BinanceSpot;
pub use futures::BinanceFutures;

/// Backwards-compatible alias.
pub type Binance = BinanceSpot;
```

Note: The `pub mod futures;` line will fail until Task 7 creates the module. Comment it out for now and uncomment in Task 7. Same for the `pub use futures::BinanceFutures;` line.

Temporary `lib.rs`:
```rust
pub mod spot;

pub use spot::BinanceSpot;

/// Backwards-compatible alias.
pub type Binance = BinanceSpot;
```

**Step 5: Update examples to use BinanceSpot (or Binance alias)**

In `crates/gateway-binance/examples/basic_rest.rs`: Change `use gateway_binance::Binance;` — this should still work via the type alias.

In `crates/gateway-binance/examples/stream_trades.rs`: Same — the `Binance` alias works.

**Step 6: Verify compile and tests**

Run: `cargo check -p gateway-binance && cargo test -p gateway-binance`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/gateway-binance/
git commit -m "refactor: move Binance spot code into spot/ submodule, rename to BinanceSpot"
```

---

## Task 5: gateway-bybit — Refactor spot into submodule

Same pattern as Task 4 but for Bybit.

**Files:**
- Modify: `crates/gateway-bybit/src/lib.rs`
- Create: `crates/gateway-bybit/src/spot/mod.rs`
- Move: `mapper.rs`, `rest.rs`, `ws.rs` → `spot/`

**Step 1: Create spot/ directory and move files**

```bash
mkdir -p crates/gateway-bybit/src/spot
mv crates/gateway-bybit/src/mapper.rs crates/gateway-bybit/src/spot/mapper.rs
mv crates/gateway-bybit/src/rest.rs crates/gateway-bybit/src/spot/rest.rs
mv crates/gateway-bybit/src/ws.rs crates/gateway-bybit/src/spot/ws.rs
```

**Step 2: Create `spot/mod.rs`**

Same structure as Binance — struct `BybitSpot`, impl `Exchange` with `ExchangeId::BybitSpot`. Copy the exact pattern from old `lib.rs` but with:
- `Bybit` → `BybitSpot`
- `ExchangeId::Bybit` → `ExchangeId::BybitSpot`

**Step 3: Update `ExchangeId::Bybit` → `ExchangeId::BybitSpot` in spot/mapper.rs, spot/rest.rs, spot/ws.rs**

In `spot/rest.rs`: Replace all `ExchangeId::Bybit` with `ExchangeId::BybitSpot`.
In `spot/ws.rs`: Replace all `ExchangeId::Bybit` with `ExchangeId::BybitSpot`.
In `spot/mapper.rs`: Replace all `ExchangeId::Bybit` with `ExchangeId::BybitSpot`.

**Step 4: Rewrite `lib.rs`**

```rust
pub mod spot;

pub use spot::BybitSpot;

/// Backwards-compatible alias.
pub type Bybit = BybitSpot;
```

**Step 5: Verify compile and tests**

Run: `cargo check -p gateway-bybit && cargo test -p gateway-bybit`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/gateway-bybit/
git commit -m "refactor: move Bybit spot code into spot/ submodule, rename to BybitSpot"
```

---

## Task 6: gateway-bitget — Refactor spot into submodule

Same pattern for Bitget.

**Files:**
- Modify: `crates/gateway-bitget/src/lib.rs`
- Create: `crates/gateway-bitget/src/spot/mod.rs`
- Move: `mapper.rs`, `rest.rs`, `ws.rs` → `spot/`

**Step 1: Create spot/ directory and move files**

```bash
mkdir -p crates/gateway-bitget/src/spot
mv crates/gateway-bitget/src/mapper.rs crates/gateway-bitget/src/spot/mapper.rs
mv crates/gateway-bitget/src/rest.rs crates/gateway-bitget/src/spot/rest.rs
mv crates/gateway-bitget/src/ws.rs crates/gateway-bitget/src/spot/ws.rs
```

**Step 2: Create `spot/mod.rs`**

Same structure — struct `BitgetSpot`, impl `Exchange` with `ExchangeId::BitgetSpot`.

**Step 3: Update `ExchangeId::Bitget` → `ExchangeId::BitgetSpot` in spot files**

**Step 4: Rewrite `lib.rs`**

```rust
pub mod spot;

pub use spot::BitgetSpot;

/// Backwards-compatible alias.
pub type Bitget = BitgetSpot;
```

**Step 5: Verify compile and tests**

Run: `cargo check -p gateway-bitget && cargo test -p gateway-bitget`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/gateway-bitget/
git commit -m "refactor: move Bitget spot code into spot/ submodule, rename to BitgetSpot"
```

---

## Task 7: gateway-manager — Fix ExchangeId references

**Files:**
- Modify: `crates/gateway-manager/examples/multi_exchange.rs`

**Step 1: Update example to use new ExchangeId variants**

In `multi_exchange.rs`, update:
- `ExchangeId::Binance` → `ExchangeId::BinanceSpot`
- `ExchangeId::Bybit` → `ExchangeId::BybitSpot`
- Imports: `use gateway_binance::Binance;` still works via alias (or switch to `BinanceSpot`)

**Step 2: Verify full workspace compile**

Run: `cargo check --workspace && cargo test --workspace`
Expected: PASS — all spot code works with new ExchangeId variants

**Step 3: Commit**

```bash
git add crates/gateway-manager/
git commit -m "fix: update gateway-manager examples for new ExchangeId variants"
```

---

## Task 8: gateway-binance — Add futures mapper

**Files:**
- Create: `crates/gateway-binance/src/futures/mapper.rs`

**Step 1: Write futures mapper**

Create `crates/gateway-binance/src/futures/mapper.rs`:

```rust
use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// Re-use spot helpers for symbol/interval conversion — same format for futures
pub use crate::spot::mapper::{
    binance_symbol_to_unified, interval_to_binance, parse_kline_row, unified_to_binance,
};

// ---------------------------------------------------------------------------
// Exchange Info (GET /fapi/v1/exchangeInfo)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceFuturesExchangeInfoRaw {
    pub symbols: Vec<BinanceFuturesSymbolRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BinanceFuturesSymbolRaw {
    pub symbol: String,
    pub status: String,
    #[serde(rename = "baseAsset")]
    pub base_asset: String,
    #[serde(rename = "quoteAsset")]
    pub quote_asset: String,
    #[serde(rename = "pricePrecision")]
    pub price_precision: u8,
    #[serde(rename = "quantityPrecision")]
    pub quantity_precision: u8,
    #[serde(default)]
    pub filters: Vec<serde_json::Value>,
}

impl BinanceFuturesExchangeInfoRaw {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .symbols
            .into_iter()
            .map(|s| {
                let status = match s.status.as_str() {
                    "TRADING" => SymbolStatus::Trading,
                    "HALT" => SymbolStatus::Halted,
                    "PRE_TRADING" => SymbolStatus::PreTrading,
                    _ => SymbolStatus::Unknown,
                };

                let mut min_qty: Option<Decimal> = None;
                let mut tick_size: Option<Decimal> = None;
                let mut min_notional: Option<Decimal> = None;

                for f in &s.filters {
                    let filter_type = f
                        .get("filterType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match filter_type {
                        "LOT_SIZE" => {
                            min_qty = f
                                .get("minQty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        "PRICE_FILTER" => {
                            tick_size = f
                                .get("tickSize")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        "MIN_NOTIONAL" => {
                            min_notional = f
                                .get("notional")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        _ => {}
                    }
                }

                SymbolInfo {
                    symbol: Symbol::new(&s.base_asset, &s.quote_asset),
                    raw_symbol: s.symbol,
                    status,
                    base_precision: s.quantity_precision,
                    quote_precision: s.price_precision,
                    min_qty,
                    min_notional,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::BinanceFutures,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /fapi/v1/depth) — same format as spot
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceFuturesOrderBookRaw {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "T")]
    pub transaction_time: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

impl BinanceFuturesOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.event_time,
            sequence: Some(self.last_update_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Trade (GET /fapi/v1/trades) — same format as spot
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceFuturesTradeRaw {
    pub id: u64,
    pub price: String,
    pub qty: String,
    pub time: u64,
    #[serde(rename = "isBuyerMaker")]
    pub is_buyer_maker: bool,
}

impl BinanceFuturesTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        Trade {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker { Side::Sell } else { Side::Buy },
            timestamp_ms: self.time,
            trade_id: Some(self.id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Ticker (GET /fapi/v1/ticker/24hr)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceFuturesTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "bidPrice", default)]
    pub bid_price: String,
    #[serde(rename = "askPrice", default)]
    pub ask_price: String,
    pub volume: String,
    #[serde(rename = "priceChangePercent")]
    pub price_change_percent: String,
    #[serde(rename = "closeTime")]
    pub close_time: u64,
}

impl BinanceFuturesTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            last_price: Decimal::from_str(&self.last_price).unwrap_or_default(),
            bid: Decimal::from_str(&self.bid_price).ok(),
            ask: Decimal::from_str(&self.ask_price).ok(),
            volume_24h: Decimal::from_str(&self.volume).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.price_change_percent).ok(),
            timestamp_ms: self.close_time,
        }
    }
}

// ---------------------------------------------------------------------------
// Premium Index / Funding Rate (GET /fapi/v1/premiumIndex)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinancePremiumIndexRaw {
    pub symbol: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
    #[serde(rename = "indexPrice")]
    pub index_price: String,
    #[serde(rename = "lastFundingRate")]
    pub last_funding_rate: String,
    #[serde(rename = "nextFundingTime")]
    pub next_funding_time: u64,
    pub time: u64,
}

impl BinancePremiumIndexRaw {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = binance_symbol_to_unified(&self.symbol);
        FundingRate {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            rate: Decimal::from_str(&self.last_funding_rate).unwrap_or_default(),
            next_funding_time_ms: self.next_funding_time,
            timestamp_ms: self.time,
        }
    }

    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = binance_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.time,
        }
    }
}

// ---------------------------------------------------------------------------
// Open Interest (GET /fapi/v1/openInterest)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceOpenInterestRaw {
    pub symbol: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
    pub time: u64,
}

impl BinanceOpenInterestRaw {
    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = binance_symbol_to_unified(&self.symbol);
        let oi = Decimal::from_str(&self.open_interest).unwrap_or_default();
        OpenInterest {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            open_interest: oi,
            open_interest_value: Decimal::ZERO, // Not available from this endpoint alone
            timestamp_ms: self.time,
        }
    }
}

// ---------------------------------------------------------------------------
// Force Orders / Liquidations (GET /fapi/v1/allForceOrders)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceForceOrderRaw {
    pub symbol: String,
    pub price: String,
    #[serde(rename = "origQty")]
    pub orig_qty: String,
    pub side: String,
    pub time: u64,
}

impl BinanceForceOrderRaw {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = binance_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "BUY" => Side::Buy,
            _ => Side::Sell,
        };
        Liquidation {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            side,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.orig_qty).unwrap_or_default(),
            timestamp_ms: self.time,
        }
    }
}

// ---------------------------------------------------------------------------
// WS types
// ---------------------------------------------------------------------------

/// WS depth update — same format as spot
#[derive(Debug, Deserialize)]
pub struct BinanceFuturesWsDepthRaw {
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

impl BinanceFuturesWsDepthRaw {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = binance_symbol_to_unified(&self.symbol);
        OrderBook {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.event_time,
            sequence: Some(self.last_update_id),
        }
    }
}

/// WS trade — same format as spot
#[derive(Debug, Deserialize)]
pub struct BinanceFuturesWsTradeRaw {
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

impl BinanceFuturesWsTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Trade {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker { Side::Sell } else { Side::Buy },
            timestamp_ms: self.trade_time,
            trade_id: Some(self.trade_id.to_string()),
        }
    }
}

/// WS kline — same format as spot
#[derive(Debug, Deserialize)]
pub struct BinanceFuturesWsKlineMsg {
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    pub k: BinanceFuturesWsKlineRaw,
}

#[derive(Debug, Deserialize)]
pub struct BinanceFuturesWsKlineRaw {
    #[serde(rename = "t")]
    pub open_time: u64,
    #[serde(rename = "T")]
    pub close_time: u64,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "x")]
    pub is_closed: bool,
}

impl BinanceFuturesWsKlineMsg {
    pub fn into_candle(self) -> Candle {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Candle {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            open: Decimal::from_str(&self.k.open).unwrap_or_default(),
            high: Decimal::from_str(&self.k.high).unwrap_or_default(),
            low: Decimal::from_str(&self.k.low).unwrap_or_default(),
            close: Decimal::from_str(&self.k.close).unwrap_or_default(),
            volume: Decimal::from_str(&self.k.volume).unwrap_or_default(),
            open_time_ms: self.k.open_time,
            close_time_ms: self.k.close_time,
            is_closed: self.k.is_closed,
        }
    }
}

/// WS markPrice stream
#[derive(Debug, Deserialize)]
pub struct BinanceWsMarkPriceRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub mark_price: String,
    #[serde(rename = "i")]
    pub index_price: String,
    #[serde(rename = "r")]
    pub funding_rate: String,
    #[serde(rename = "T")]
    pub next_funding_time: u64,
    #[serde(rename = "E")]
    pub event_time: u64,
}

impl BinanceWsMarkPriceRaw {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = binance_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.event_time,
        }
    }
}

/// WS forceOrder (liquidation) stream
#[derive(Debug, Deserialize)]
pub struct BinanceWsForceOrderMsg {
    #[serde(rename = "E")]
    pub event_time: u64,
    pub o: BinanceWsForceOrderRaw,
}

#[derive(Debug, Deserialize)]
pub struct BinanceWsForceOrderRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "S")]
    pub side: String,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub qty: String,
    #[serde(rename = "T")]
    pub trade_time: u64,
}

impl BinanceWsForceOrderMsg {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = binance_symbol_to_unified(&self.o.symbol);
        let side = match self.o.side.as_str() {
            "BUY" => Side::Buy,
            _ => Side::Sell,
        };
        Liquidation {
            exchange: ExchangeId::BinanceFutures,
            symbol,
            side,
            price: Decimal::from_str(&self.o.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.o.qty).unwrap_or_default(),
            timestamp_ms: self.o.trade_time,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn parse_levels(raw: &[[String; 2]]) -> Vec<Level> {
    raw.iter()
        .filter_map(|pair| {
            let price = Decimal::from_str(&pair[0]).ok()?;
            let qty = Decimal::from_str(&pair[1]).ok()?;
            Some(Level::new(price, qty))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_premium_index_to_funding_rate() {
        let raw: BinancePremiumIndexRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "markPrice": "50000.00",
                "indexPrice": "49995.00",
                "lastFundingRate": "0.0001",
                "nextFundingTime": 1700000000000,
                "time": 1699999000000
            }"#,
        ).unwrap();

        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::BinanceFutures);
        assert_eq!(fr.symbol.base, "BTC");
        assert_eq!(fr.rate, dec!(0.0001));
        assert_eq!(fr.next_funding_time_ms, 1700000000000);
    }

    #[test]
    fn test_premium_index_to_mark_price() {
        let raw: BinancePremiumIndexRaw = serde_json::from_str(
            r#"{
                "symbol": "ETHUSDT",
                "markPrice": "2000.50",
                "indexPrice": "2000.00",
                "lastFundingRate": "0.0002",
                "nextFundingTime": 1700000000000,
                "time": 1699999000000
            }"#,
        ).unwrap();

        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::BinanceFutures);
        assert_eq!(mp.symbol.base, "ETH");
        assert_eq!(mp.mark_price, dec!(2000.50));
        assert_eq!(mp.index_price, dec!(2000.00));
    }

    #[test]
    fn test_open_interest_conversion() {
        let raw: BinanceOpenInterestRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "openInterest": "12345.678",
                "time": 1700000000000
            }"#,
        ).unwrap();

        let oi = raw.into_open_interest();
        assert_eq!(oi.exchange, ExchangeId::BinanceFutures);
        assert_eq!(oi.open_interest, dec!(12345.678));
    }

    #[test]
    fn test_force_order_to_liquidation() {
        let raw: BinanceForceOrderRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "price": "50000.00",
                "origQty": "0.5",
                "side": "SELL",
                "time": 1700000000000
            }"#,
        ).unwrap();

        let liq = raw.into_liquidation();
        assert_eq!(liq.exchange, ExchangeId::BinanceFutures);
        assert_eq!(liq.side, Side::Sell);
        assert_eq!(liq.price, dec!(50000.00));
        assert_eq!(liq.qty, dec!(0.5));
    }

    #[test]
    fn test_ws_mark_price_conversion() {
        let raw: BinanceWsMarkPriceRaw = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "p": "50000.00",
                "i": "49995.00",
                "r": "0.0001",
                "T": 1700000000000,
                "E": 1699999000000
            }"#,
        ).unwrap();

        let mp = raw.into_mark_price();
        assert_eq!(mp.mark_price, dec!(50000.00));
        assert_eq!(mp.index_price, dec!(49995.00));
    }

    #[test]
    fn test_ws_force_order_conversion() {
        let raw: BinanceWsForceOrderMsg = serde_json::from_str(
            r#"{
                "E": 1700000000000,
                "o": {
                    "s": "ETHUSDT",
                    "S": "BUY",
                    "p": "2000.00",
                    "q": "1.5",
                    "T": 1700000000000
                }
            }"#,
        ).unwrap();

        let liq = raw.into_liquidation();
        assert_eq!(liq.symbol.base, "ETH");
        assert_eq!(liq.side, Side::Buy);
        assert_eq!(liq.price, dec!(2000.00));
    }

    #[test]
    fn test_futures_ticker_conversion() {
        let raw: BinanceFuturesTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPrice": "50000.00",
                "bidPrice": "49999.00",
                "askPrice": "50001.00",
                "volume": "12345.678",
                "priceChangePercent": "2.5",
                "closeTime": 1700000000000
            }"#,
        ).unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::BinanceFutures);
        assert_eq!(ticker.last_price, dec!(50000.00));
    }
}
```

**Step 2: Verify compile**

Run: `cargo check -p gateway-binance`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/gateway-binance/src/futures/
git commit -m "feat: add Binance futures mapper with tests"
```

---

## Task 9: gateway-binance — Add futures REST client

**Files:**
- Create: `crates/gateway-binance/src/futures/rest.rs`

**Step 1: Write futures REST client**

Create `crates/gateway-binance/src/futures/rest.rs`. Follow the exact same pattern as `spot/rest.rs` but:
- Base URL: `https://fapi.binance.com`
- Endpoints: `/fapi/v1/depth`, `/fapi/v1/trades`, `/fapi/v1/klines`, `/fapi/v1/ticker/24hr`
- Additional futures endpoints: `/fapi/v1/premiumIndex`, `/fapi/v1/openInterest`, `/fapi/v1/allForceOrders`
- Use `BinanceFutures*` mapper types from `futures/mapper.rs`
- All error references use `ExchangeId::BinanceFutures`

The REST client follows the same pattern as spot — no response wrapper for Binance, direct JSON parsing.

Add these futures-specific methods:
- `pub async fn premium_index(&self, symbol: &Symbol) -> Result<BinancePremiumIndexRaw>`
- `pub async fn open_interest(&self, symbol: &Symbol) -> Result<BinanceOpenInterestRaw>`
- `pub async fn force_orders(&self, symbol: &Symbol, limit: u16) -> Result<Vec<BinanceForceOrderRaw>>`

**Step 2: Verify compile**

Run: `cargo check -p gateway-binance`

**Step 3: Commit**

```bash
git add crates/gateway-binance/src/futures/rest.rs
git commit -m "feat: add Binance futures REST client"
```

---

## Task 10: gateway-binance — Add futures WebSocket

**Files:**
- Create: `crates/gateway-binance/src/futures/ws.rs`

**Step 1: Write futures WebSocket module**

Create `crates/gateway-binance/src/futures/ws.rs`. Follow the exact same pattern as `spot/ws.rs` but:
- WS URL: `wss://fstream.binance.com/ws`
- Combined WS URL: `wss://fstream.binance.com/stream`
- Uses futures mapper types
- Same stream names as spot (btcusdt@depth@100ms, btcusdt@trade, btcusdt@kline_1m)
- Add `stream_mark_price()` → subscribes to `btcusdt@markPrice@1s`
- Add `stream_liquidations()` → subscribes to `btcusdt@forceOrder`

The `subscribe_and_stream` helper can be duplicated (it has different WS URLs).

**Step 2: Verify compile**

Run: `cargo check -p gateway-binance`

**Step 3: Commit**

```bash
git add crates/gateway-binance/src/futures/ws.rs
git commit -m "feat: add Binance futures WebSocket streams"
```

---

## Task 11: gateway-binance — Add BinanceFutures struct

**Files:**
- Create: `crates/gateway-binance/src/futures/mod.rs`
- Modify: `crates/gateway-binance/src/lib.rs` (add futures module)

**Step 1: Create `futures/mod.rs`**

```rust
pub mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct BinanceFutures {
    config: ExchangeConfig,
    rest: rest::BinanceFuturesRest,
}

impl BinanceFutures {
    pub fn new(config: ExchangeConfig) -> Self {
        let rest = rest::BinanceFuturesRest::new(&config);
        Self { config, rest }
    }

    pub fn public() -> Self {
        Self::new(ExchangeConfig::default())
    }
}

#[async_trait]
impl Exchange for BinanceFutures {
    fn id(&self) -> ExchangeId {
        ExchangeId::BinanceFutures
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

    async fn stream_orderbooks_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<OrderBook>> {
        ws::stream_orderbooks_combined(&self.config, symbols).await
    }

    async fn stream_trades_batch(&self, symbols: &[Symbol]) -> Result<BoxStream<Trade>> {
        ws::stream_trades_combined(&self.config, symbols).await
    }
}

#[async_trait]
impl FuturesExchange for BinanceFutures {
    async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRate> {
        let raw = self.rest.premium_index(symbol).await?;
        Ok(raw.into_funding_rate())
    }

    async fn mark_price(&self, symbol: &Symbol) -> Result<MarkPrice> {
        let raw = self.rest.premium_index(symbol).await?;
        Ok(raw.into_mark_price())
    }

    async fn open_interest(&self, symbol: &Symbol) -> Result<OpenInterest> {
        let raw = self.rest.open_interest(symbol).await?;
        Ok(raw.into_open_interest())
    }

    async fn liquidations(&self, symbol: &Symbol, limit: u16) -> Result<Vec<Liquidation>> {
        let raw = self.rest.force_orders(symbol, limit).await?;
        Ok(raw.into_iter().map(|r| r.into_liquidation()).collect())
    }

    async fn stream_mark_price(&self, symbol: &Symbol) -> Result<BoxStream<MarkPrice>> {
        ws::stream_mark_price(&self.config, symbol).await
    }

    async fn stream_liquidations(&self, symbol: &Symbol) -> Result<BoxStream<Liquidation>> {
        ws::stream_liquidations(&self.config, symbol).await
    }
}
```

**Step 2: Update `lib.rs` to include futures module**

```rust
pub mod spot;
pub mod futures;

pub use spot::BinanceSpot;
pub use futures::BinanceFutures;

/// Backwards-compatible alias.
pub type Binance = BinanceSpot;
```

**Step 3: Verify compile and tests**

Run: `cargo check -p gateway-binance && cargo test -p gateway-binance`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/gateway-binance/
git commit -m "feat: add BinanceFutures with Exchange + FuturesExchange impl"
```

---

## Task 12: gateway-bybit — Add futures submodule

**Files:**
- Create: `crates/gateway-bybit/src/futures/mod.rs`
- Create: `crates/gateway-bybit/src/futures/mapper.rs`
- Create: `crates/gateway-bybit/src/futures/rest.rs`
- Create: `crates/gateway-bybit/src/futures/ws.rs`
- Modify: `crates/gateway-bybit/src/lib.rs`

Bybit V5 API makes this the simplest — same endpoints as spot, but `category=linear` instead of `category=spot`. Same response format.

**Key differences from spot:**
- REST: All URLs use `category=linear` instead of `category=spot`
- WS URL: `wss://stream.bybit.com/v5/public/linear` instead of `spot`
- Tickers include `fundingRate`, `markPrice`, `openInterest` fields
- Additional endpoints:
  - GET `/v5/market/funding/history?category=linear&symbol=BTCUSDT` — funding rate history
  - GET `/v5/market/open-interest?category=linear&symbol=BTCUSDT&intervalTime=5min` — OI
- WS topics:
  - `tickers.BTCUSDT` — includes funding rate, mark price
  - `liquidation.BTCUSDT` — liquidation events

**Mapper**: Re-use `crate::spot::mapper` symbol/interval helpers. Add new types for:
- `BybitLinearTickerRaw` — extends spot ticker with `fundingRate`, `markPrice`, `indexPrice`, `openInterest` fields
- `BybitFundingHistoryResult` / `BybitFundingHistoryRaw`
- `BybitOpenInterestResult` / `BybitOpenInterestRaw`
- `BybitWsLiquidation`

**REST**: Same as spot/rest.rs but all `category=spot` → `category=linear`.
Additional methods: `funding_rate()`, `open_interest()`.

**WS**: Same as spot/ws.rs but URL changes to linear.
Additional streams: `stream_mark_price()` (via tickers topic), `stream_liquidations()`.

**Step 1:** Create all four files following the patterns from Binance futures + Bybit spot.

**Step 2: Update `lib.rs`**

```rust
pub mod spot;
pub mod futures;

pub use spot::BybitSpot;
pub use futures::BybitFutures;

/// Backwards-compatible alias.
pub type Bybit = BybitSpot;
```

**Step 3: Verify compile and tests**

Run: `cargo check -p gateway-bybit && cargo test -p gateway-bybit`

**Step 4: Commit**

```bash
git add crates/gateway-bybit/
git commit -m "feat: add BybitFutures with Exchange + FuturesExchange impl"
```

---

## Task 13: gateway-bitget — Add futures submodule

**Files:**
- Create: `crates/gateway-bitget/src/futures/mod.rs`
- Create: `crates/gateway-bitget/src/futures/mapper.rs`
- Create: `crates/gateway-bitget/src/futures/rest.rs`
- Create: `crates/gateway-bitget/src/futures/ws.rs`
- Modify: `crates/gateway-bitget/src/lib.rs`

**Key differences from spot:**
- REST path: `/api/v2/mix/market/...` instead of `/api/v2/spot/market/...`
- Instruments: `/api/v2/mix/market/contracts?productType=USDT-FUTURES`
- Orderbook: `/api/v2/mix/market/merge-depth?productType=USDT-FUTURES&symbol=BTCUSDT`
- Trades: `/api/v2/mix/market/fills?productType=USDT-FUTURES&symbol=BTCUSDT`
- Candles: `/api/v2/mix/market/candles?productType=USDT-FUTURES&symbol=BTCUSDT&granularity=1m`
- Ticker: `/api/v2/mix/market/ticker?productType=USDT-FUTURES&symbol=BTCUSDT`
- Funding: `/api/v2/mix/market/current-fund-rate?productType=USDT-FUTURES&symbol=BTCUSDT`
- OI: `/api/v2/mix/market/open-interest?productType=USDT-FUTURES&symbol=BTCUSDT`

- WS: `instType: "USDT-FUTURES"` instead of `"SPOT"` in subscription args

**Mapper**: Re-use `crate::spot::mapper` helpers. Add Bitget futures-specific types:
- `BitgetMixSymbolRaw` — similar to spot but from contracts endpoint
- `BitgetMixTickerRaw` — includes fundingRate, markPrice, openInterest
- `BitgetFundingRateRaw`
- `BitgetOpenInterestRaw`
- WS: Same types as spot but with `ExchangeId::BitgetFutures`

**Step 1:** Create all four files following the patterns from spot Bitget + Binance futures.

**Step 2: Update `lib.rs`**

```rust
pub mod spot;
pub mod futures;

pub use spot::BitgetSpot;
pub use futures::BitgetFutures;

/// Backwards-compatible alias.
pub type Bitget = BitgetSpot;
```

**Step 3: Verify compile and tests**

Run: `cargo check -p gateway-bitget && cargo test -p gateway-bitget`

**Step 4: Commit**

```bash
git add crates/gateway-bitget/
git commit -m "feat: add BitgetFutures with Exchange + FuturesExchange impl"
```

---

## Task 14: gateway-manager — Add FuturesExchange support

**Files:**
- Modify: `crates/gateway-manager/src/lib.rs`

**Step 1: Add dual storage and futures methods**

Update `GatewayManager` to store futures exchanges separately:

```rust
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

    /// Register an exchange (spot or futures).
    pub fn register(&mut self, exchange: impl Exchange) -> &mut Self {
        let id = exchange.id();
        self.exchanges.insert(id, Arc::new(exchange));
        self
    }

    /// Register a futures exchange (also registers as a regular Exchange).
    pub fn register_futures(&mut self, exchange: impl FuturesExchange) -> &mut Self {
        let id = exchange.id();
        let arc: Arc<dyn FuturesExchange> = Arc::new(exchange);
        // FuturesExchange: Exchange, so we can clone as dyn Exchange too
        self.exchanges.insert(id, arc.clone());
        self.futures_exchanges.insert(id, arc);
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
            handles.push(tokio::spawn(async move {
                (id, ex.funding_rate(&sym).await)
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
```

**Step 2: Update multi_exchange.rs example**

Update `crates/gateway-manager/examples/multi_exchange.rs` to also register futures exchanges and demo fetching funding rates.

**Step 3: Verify compile**

Run: `cargo check --workspace && cargo test --workspace`

**Step 4: Commit**

```bash
git add crates/gateway-manager/
git commit -m "feat: add FuturesExchange support to GatewayManager"
```

---

## Task 15: Add futures examples

**Files:**
- Create: `crates/gateway-binance/examples/futures_rest.rs`
- Create: `crates/gateway-bybit/examples/futures_rest.rs`
- Create: `crates/gateway-bitget/examples/futures_rest.rs`

Each example should demonstrate:
1. Create `{Exchange}Futures::public()`
2. Fetch exchange info (number of futures pairs)
3. Fetch BTC/USDT futures ticker
4. Fetch funding rate
5. Fetch mark price
6. Fetch open interest
7. Fetch orderbook (top 5 levels)
8. Fetch recent trades

Follow the exact pattern of existing `basic_rest.rs` examples.

**Step 1: Write all three examples**

**Step 2: Verify they compile**

Run: `cargo check --workspace --examples`

**Step 3: Commit**

```bash
git add crates/gateway-binance/examples/ crates/gateway-bybit/examples/ crates/gateway-bitget/examples/
git commit -m "feat: add futures REST examples for all exchanges"
```

---

## Task 16: Final verification

**Step 1: Run full workspace check**

Run: `cargo check --workspace`
Expected: PASS

**Step 2: Run all tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 3: Run clippy**

Run: `cargo clippy --workspace`
Expected: No errors (warnings ok)

**Step 4: Verify examples compile**

Run: `cargo check --workspace --examples`
Expected: PASS

**Step 5: Final commit if any fixes needed**
