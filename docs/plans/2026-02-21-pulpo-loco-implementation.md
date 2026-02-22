# pulpo_loco Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust workspace providing unified REST + WebSocket access to Binance and Bybit crypto exchanges.

**Architecture:** 4-crate workspace: gateway-core (types/traits), gateway-binance, gateway-bybit, gateway-manager (multiplexer). Each exchange crate has REST client (reqwest), WS client (tokio-tungstenite), and JSON mapper.

**Tech Stack:** Rust 2021, tokio, reqwest, tokio-tungstenite, serde/serde_json, rust_decimal, async-trait, thiserror, tracing, futures

---

## API Reference (researched)

### Binance Spot API
- Base REST: `https://api.binance.com`
- Base WS: `wss://stream.binance.com:9443/ws/` (single) / `wss://stream.binance.com:9443/stream?streams=` (combined)
- `GET /api/v3/exchangeInfo` — symbol info
- `GET /api/v3/depth?symbol=BTCUSDT&limit=100` — orderbook
- `GET /api/v3/trades?symbol=BTCUSDT&limit=500` — recent trades
- `GET /api/v3/klines?symbol=BTCUSDT&interval=1h&limit=500` — klines
- `GET /api/v3/ticker/24hr` — all tickers (no symbol param)
- `GET /api/v3/ticker/24hr?symbol=BTCUSDT` — single ticker
- WS depth: `<symbol>@depth@100ms` — diff depth stream
- WS trade: `<symbol>@trade` — real-time trades
- WS kline: `<symbol>@kline_<interval>` — candle stream
- WS subscribe: `{"method":"SUBSCRIBE","params":["btcusdt@trade"],"id":1}`

### Bybit V5 API
- Base REST: `https://api.bybit.com`
- Base WS Spot: `wss://stream.bybit.com/v5/public/spot`
- `GET /v5/market/instruments-info?category=spot` — symbol info
- `GET /v5/market/orderbook?category=spot&symbol=BTCUSDT&limit=50` — orderbook
- `GET /v5/market/recent-trade?category=spot&symbol=BTCUSDT&limit=60` — trades
- `GET /v5/market/kline?category=spot&symbol=BTCUSDT&interval=60&limit=200` — klines
- `GET /v5/market/tickers?category=spot` — all tickers
- `GET /v5/market/tickers?category=spot&symbol=BTCUSDT` — single ticker
- WS orderbook: `orderbook.50.BTCUSDT` (depths: 1, 50, 200)
- WS trade: `publicTrade.BTCUSDT`
- WS kline: `kline.60.BTCUSDT`
- WS ticker: `tickers.BTCUSDT`
- WS subscribe: `{"op":"subscribe","args":["orderbook.50.BTCUSDT"]}`
- Response wrapper: `{"retCode":0,"retMsg":"OK","result":{...}}`

---

### Task 1: Workspace Scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/gateway-core/Cargo.toml`
- Create: `crates/gateway-core/src/lib.rs` (empty placeholder)
- Create: `crates/gateway-binance/Cargo.toml`
- Create: `crates/gateway-binance/src/lib.rs` (empty placeholder)
- Create: `crates/gateway-bybit/Cargo.toml`
- Create: `crates/gateway-bybit/src/lib.rs` (empty placeholder)
- Create: `crates/gateway-manager/Cargo.toml`
- Create: `crates/gateway-manager/src/lib.rs` (empty placeholder)

**Step 1: Create directory structure**

```bash
mkdir -p crates/{gateway-core,gateway-binance,gateway-bybit,gateway-manager}/src
```

**Step 2: Write workspace root Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/gateway-core",
    "crates/gateway-binance",
    "crates/gateway-bybit",
    "crates/gateway-manager",
]
```

**Step 3: Write gateway-core/Cargo.toml**

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

**Step 4: Write gateway-binance/Cargo.toml**

```toml
[package]
name = "gateway-binance"
version = "0.1.0"
edition = "2021"

[dependencies]
gateway-core = { path = "../gateway-core" }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
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

**Step 5: Write gateway-bybit/Cargo.toml**

```toml
[package]
name = "gateway-bybit"
version = "0.1.0"
edition = "2021"

[dependencies]
gateway-core = { path = "../gateway-core" }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
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

**Step 6: Write gateway-manager/Cargo.toml**

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

gateway-binance = { path = "../gateway-binance", optional = true }
gateway-bybit = { path = "../gateway-bybit", optional = true }

[features]
default = ["binance", "bybit"]
binance = ["dep:gateway-binance"]
bybit = ["dep:gateway-bybit"]
all = ["binance", "bybit"]
```

**Step 7: Write placeholder lib.rs for all crates**

Each `src/lib.rs` starts empty.

**Step 8: Verify workspace compiles**

Run: `cargo check`
Expected: compiles with no errors

**Step 9: Init git + commit**

```bash
git init
git add -A
git commit -m "chore: scaffold pulpo_loco workspace with 4 crates"
```

---

### Task 2: gateway-core — Types & Error

**Files:**
- Create: `crates/gateway-core/src/types.rs`
- Create: `crates/gateway-core/src/error.rs`
- Modify: `crates/gateway-core/src/lib.rs`

**Step 1: Write types.rs**

All unified types: `ExchangeId`, `Symbol`, `Level`, `OrderBook`, `Side`, `Trade`, `Candle`, `Ticker`, `Interval`, `SymbolInfo`, `SymbolStatus`, `ExchangeInfo`. Exactly as in project.md spec.

**Step 2: Write error.rs**

`GatewayError` enum with variants: Rest, WebSocket, RateLimited, SymbolNotFound, Auth, Parse, Disconnected, Other. Using `thiserror`.

**Step 3: Update lib.rs to declare modules**

```rust
pub mod error;
pub mod types;
pub use error::{GatewayError, Result};
pub use types::*;
```

**Step 4: Verify**

Run: `cargo check -p gateway-core`
Expected: compiles

**Step 5: Commit**

```bash
git add crates/gateway-core/src/{types.rs,error.rs,lib.rs}
git commit -m "feat(core): add unified types and error"
```

---

### Task 3: gateway-core — Config, Stream, Traits

**Files:**
- Create: `crates/gateway-core/src/config.rs`
- Create: `crates/gateway-core/src/stream.rs`
- Create: `crates/gateway-core/src/traits.rs`
- Modify: `crates/gateway-core/src/lib.rs`

**Step 1: Write config.rs**

`WsConfig`, `RestConfig`, `ExchangeConfig` with Default impls. As in project.md spec.

**Step 2: Write stream.rs**

`StreamEvent` enum, `BoxStream<T>` type alias, `Subscription` handle. As in project.md spec.

**Step 3: Write traits.rs**

`Exchange` trait with all REST + WS methods, default batch implementations using `SelectAll`. `ExchangeTrading` trait with `Balance`, `NewOrder`, `OrderType`, `OrderResponse`, `Order`. As in project.md spec.

**Step 4: Update lib.rs**

```rust
pub mod config;
pub mod error;
pub mod stream;
pub mod traits;
pub mod types;

pub use config::*;
pub use error::{GatewayError, Result};
pub use stream::*;
pub use traits::*;
pub use types::*;
```

**Step 5: Verify**

Run: `cargo check -p gateway-core`
Expected: compiles

**Step 6: Commit**

```bash
git add crates/gateway-core/src/
git commit -m "feat(core): add config, stream, traits"
```

---

### Task 4: gateway-binance — Mapper

**Files:**
- Create: `crates/gateway-binance/src/mapper.rs`

**Step 1: Write mapper.rs**

Binance raw JSON structs (serde Deserialize):
- `BinanceExchangeInfoRaw` with `symbols: Vec<BinanceSymbolRaw>` — maps `GET /api/v3/exchangeInfo`
  - `BinanceSymbolRaw`: `symbol`, `status`, `baseAsset`, `quoteAsset`, `baseAssetPrecision`, `quoteAssetPrecision`, `filters` (Vec<serde_json::Value>)
- `BinanceOrderBookRaw` — `lastUpdateId`, `bids: Vec<[String;2]>`, `asks: Vec<[String;2]>`
- `BinanceTradeRaw` — `id`, `price`, `qty`, `time`, `isBuyerMaker`
- `BinanceTickerRaw` — `symbol`, `lastPrice`, `bidPrice`, `askPrice`, `volume`, `priceChangePercent`, `closeTime`
- `BinanceWsDepthRaw` — WS depth update: `s`, `b`, `a`, `E`, `u`
- `BinanceWsTradeRaw` — WS trade: `s`, `p`, `q`, `T`, `t`, `m`
- `BinanceWsKlineRaw` — WS kline: `e`, `E`, `s`, `k{t,T,s,i,o,c,h,l,v,n,x,q}`

Conversion functions:
- `parse_levels(&[[String;2]]) -> Vec<Level>`
- `impl Into<OrderBook>` for raw types
- `unified_to_binance(symbol) -> String` (BTCUSDT)
- `binance_symbol_to_unified(raw) -> Symbol`
- `interval_to_binance(Interval) -> &str`

**Step 2: Write unit tests for mapper**

```rust
#[cfg(test)]
mod tests {
    #[test] fn test_unified_to_binance() { ... }
    #[test] fn test_binance_to_unified() { ... }
    #[test] fn test_parse_levels() { ... }
    #[test] fn test_interval_mapping() { ... }
}
```

**Step 3: Verify**

Run: `cargo test -p gateway-binance`
Expected: all tests pass

**Step 4: Commit**

```bash
git add crates/gateway-binance/
git commit -m "feat(binance): add JSON mapper with tests"
```

---

### Task 5: gateway-binance — REST Client

**Files:**
- Create: `crates/gateway-binance/src/rest.rs`
- Modify: `crates/gateway-binance/src/lib.rs`

**Step 1: Write rest.rs**

`BinanceRest` struct with `reqwest::Client` and base_url `https://api.binance.com`.

Methods:
- `exchange_info()` — GET /api/v3/exchangeInfo → parse BinanceExchangeInfoRaw → ExchangeInfo
- `orderbook(symbol, depth)` — GET /api/v3/depth?symbol={}&limit={} → OrderBook
- `trades(symbol, limit)` — GET /api/v3/trades?symbol={}&limit={} → Vec<Trade>
- `candles(symbol, interval, limit)` — GET /api/v3/klines?symbol={}&interval={}&limit={} → Vec<Candle>
- `ticker(symbol)` — GET /api/v3/ticker/24hr?symbol={} → Ticker
- `all_tickers()` — GET /api/v3/ticker/24hr → Vec<Ticker>

Error mapping: reqwest errors → GatewayError::Rest, parse errors → GatewayError::Parse.

**Step 2: Update lib.rs with module declarations**

```rust
mod mapper;
mod rest;
pub mod ws;

use async_trait::async_trait;
use gateway_core::*;

pub struct Binance { config: ExchangeConfig, rest: rest::BinanceRest }
// impl Exchange for Binance (REST methods only, WS stubs with todo!())
```

**Step 3: Verify**

Run: `cargo check -p gateway-binance`
Expected: compiles

**Step 4: Commit**

```bash
git add crates/gateway-binance/
git commit -m "feat(binance): add REST client"
```

---

### Task 6: gateway-binance — WebSocket Client

**Files:**
- Create: `crates/gateway-binance/src/ws.rs`
- Modify: `crates/gateway-binance/src/lib.rs` (remove todo!() stubs)

**Step 1: Write ws.rs**

Core WS helper: `connect_and_subscribe(url, streams) -> BoxStream<serde_json::Value>`
- Connects to `wss://stream.binance.com:9443/ws/`
- Sends SUBSCRIBE message: `{"method":"SUBSCRIBE","params":[...],"id":1}`
- Returns parsed JSON stream with reconnect loop on disconnect

Public functions:
- `stream_orderbook(config, symbol) -> Result<BoxStream<OrderBook>>`
  - stream = `{symbol_lower}@depth@100ms`
  - Parse each message as `BinanceWsDepthRaw`, convert to `OrderBook`
- `stream_trades(config, symbol) -> Result<BoxStream<Trade>>`
  - stream = `{symbol_lower}@trade`
  - Parse as `BinanceWsTradeRaw`, convert to `Trade`
- `stream_candles(config, symbol, interval) -> Result<BoxStream<Candle>>`
  - stream = `{symbol_lower}@kline_{interval}`
  - Parse as `BinanceWsKlineRaw`, convert to `Candle`
- `stream_orderbooks_combined(config, symbols) -> Result<BoxStream<OrderBook>>`
  - Combined stream URL: `wss://stream.binance.com:9443/stream?streams=sym1@depth@100ms/sym2@depth@100ms`
  - Messages wrapped in `{"stream":"...","data":{...}}`
- `stream_trades_combined(config, symbols) -> Result<BoxStream<Trade>>`
  - Same combined approach

**Step 2: Complete Exchange impl in lib.rs**

Replace all WS `todo!()` with real calls to ws module.

**Step 3: Verify**

Run: `cargo check -p gateway-binance`
Expected: compiles

**Step 4: Commit**

```bash
git add crates/gateway-binance/
git commit -m "feat(binance): add WebSocket client"
```

---

### Task 7: gateway-bybit — Mapper

**Files:**
- Create: `crates/gateway-bybit/src/mapper.rs`

**Step 1: Write mapper.rs**

Bybit V5 raw JSON structs:
- `BybitResponse<T>` — wrapper: `retCode: i32, retMsg: String, result: T`
- `BybitInstrumentsResult` — `category: String, list: Vec<BybitInstrumentRaw>`
  - `BybitInstrumentRaw`: `symbol`, `baseCoin`, `quoteCoin`, `status`, `lotSizeFilter{basePrecision, quotePrecision, minOrderQty}`, `priceFilter{tickSize}`
- `BybitOrderBookResult` — `s: String, b: Vec<[String;2]>, a: Vec<[String;2]>, u: u64, ts: u64`
- `BybitTradeRaw` — `execId`, `symbol`, `price`, `size`, `side`, `time`
- `BybitTickerRaw` — `symbol`, `lastPrice`, `bid1Price`, `ask1Price`, `volume24h`, `price24hPcnt`, `highPrice24h`, `lowPrice24h`, `turnover24h`
- `BybitKlineRaw` — array: `[startTime, open, high, low, close, volume, turnover]`
- WS types:
  - `BybitWsMessage<T>` — `topic: String, type: String, ts: u64, data: T`
  - `BybitWsOrderBook` — same as REST orderbook result
  - `BybitWsTrade` — `T: u64, s: String, S: String, v: String, p: String, i: String`
  - `BybitWsKline` — `start: u64, end: u64, interval: String, open: String, close: String, high: String, low: String, volume: String, confirm: bool`
  - `BybitWsTicker` — `symbol`, `lastPrice`, `bid1Price`, `ask1Price`, `volume24h`, `price24hPcnt`

Conversion functions:
- `unified_to_bybit(symbol) -> String` — just "BTCUSDT" (same as binance)
- `bybit_symbol_to_unified(raw) -> Symbol` — same heuristic
- `interval_to_bybit(Interval) -> &str` — "1" for M1, "5" for M5, "60" for H1, "240" for H4, "D" for D1, "W" for W1
- `bybit_status_to_unified(status: &str) -> SymbolStatus` — "Trading" -> Trading

**Step 2: Write unit tests**

```rust
#[cfg(test)]
mod tests {
    #[test] fn test_interval_to_bybit() { ... }
    #[test] fn test_symbol_conversion() { ... }
    #[test] fn test_parse_ticker() { ... }
}
```

**Step 3: Verify**

Run: `cargo test -p gateway-bybit`
Expected: all tests pass

**Step 4: Commit**

```bash
git add crates/gateway-bybit/
git commit -m "feat(bybit): add JSON mapper with tests"
```

---

### Task 8: gateway-bybit — REST Client

**Files:**
- Create: `crates/gateway-bybit/src/rest.rs`
- Modify: `crates/gateway-bybit/src/lib.rs`

**Step 1: Write rest.rs**

`BybitRest` struct with `reqwest::Client` and base_url `https://api.bybit.com`.

Methods:
- `exchange_info()` — GET /v5/market/instruments-info?category=spot → ExchangeInfo
- `orderbook(symbol, depth)` — GET /v5/market/orderbook?category=spot&symbol={}&limit={} → OrderBook
- `trades(symbol, limit)` — GET /v5/market/recent-trade?category=spot&symbol={}&limit={} → Vec<Trade>
  - Note: Bybit spot limit max 60
- `candles(symbol, interval, limit)` — GET /v5/market/kline?category=spot&symbol={}&interval={}&limit={} → Vec<Candle>
  - Note: Bybit returns candles in reverse order (newest first), must reverse
- `ticker(symbol)` — GET /v5/market/tickers?category=spot&symbol={} → Ticker
- `all_tickers()` — GET /v5/market/tickers?category=spot → Vec<Ticker>

All responses wrapped in `BybitResponse<T>`, check `retCode == 0`.

**Step 2: Update lib.rs**

```rust
mod mapper;
mod rest;
pub mod ws;

pub struct Bybit { config: ExchangeConfig, rest: rest::BybitRest }
impl Bybit { pub fn new(config) -> Self; pub fn public() -> Self; }
// impl Exchange for Bybit (REST methods, WS todo!())
```

**Step 3: Verify**

Run: `cargo check -p gateway-bybit`
Expected: compiles

**Step 4: Commit**

```bash
git add crates/gateway-bybit/
git commit -m "feat(bybit): add REST client"
```

---

### Task 9: gateway-bybit — WebSocket Client

**Files:**
- Create: `crates/gateway-bybit/src/ws.rs`
- Modify: `crates/gateway-bybit/src/lib.rs`

**Step 1: Write ws.rs**

Core WS helper: `connect_and_subscribe(topics) -> BoxStream<serde_json::Value>`
- Connects to `wss://stream.bybit.com/v5/public/spot`
- Sends: `{"op":"subscribe","args":["topic1","topic2"]}`
- Handles ping/pong (Bybit sends `{"op":"ping"}`, respond with `{"op":"pong"}`)
- Returns parsed JSON stream

Public functions:
- `stream_orderbook(config, symbol) -> Result<BoxStream<OrderBook>>`
  - topic = `orderbook.50.{SYMBOL}`
  - Parse snapshot/delta as `BybitWsOrderBook`
- `stream_trades(config, symbol) -> Result<BoxStream<Trade>>`
  - topic = `publicTrade.{SYMBOL}`
  - Data is array of trades, flatten
- `stream_candles(config, symbol, interval) -> Result<BoxStream<Candle>>`
  - topic = `kline.{interval}.{SYMBOL}`
- Batch functions: subscribe to multiple topics in one connection

**Step 2: Complete Exchange impl in lib.rs**

Replace WS `todo!()` with real calls.

**Step 3: Verify**

Run: `cargo check -p gateway-bybit`
Expected: compiles

**Step 4: Commit**

```bash
git add crates/gateway-bybit/
git commit -m "feat(bybit): add WebSocket client"
```

---

### Task 10: gateway-manager

**Files:**
- Create: `crates/gateway-manager/src/lib.rs`

**Step 1: Write lib.rs**

`GatewayManager` with:
- `exchanges: HashMap<ExchangeId, Arc<dyn Exchange>>`
- `new()`, `register()`, `get()`, `all()`
- `all_tickers_everywhere()` — parallel tokio::spawn for all exchanges
- `stream_trades_multi(pairs)` — SelectAll merge

As in project.md spec.

**Step 2: Verify**

Run: `cargo check -p gateway-manager`
Expected: compiles

**Step 3: Commit**

```bash
git add crates/gateway-manager/
git commit -m "feat(manager): add exchange multiplexer"
```

---

### Task 11: Rename project.md + Final Verification

**Files:**
- Modify: `project.md` — replace all "crypto-gateway" with "pulpo_loco"

**Step 1: Update project.md title and references**

Replace "crypto-gateway" → "pulpo_loco" everywhere in the file.

**Step 2: Full build**

Run: `cargo build`
Expected: compiles with no errors

**Step 3: Run all tests**

Run: `cargo test`
Expected: all tests pass

**Step 4: Commit**

```bash
git add project.md
git commit -m "chore: rename project to pulpo_loco"
```

---
