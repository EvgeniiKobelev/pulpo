<div align="center">

<img src="logo.png" alt="Pulpo Loco" width="200">

# Pulpo Loco

**Unified Rust gateway for cryptocurrency exchanges**

One trait. Multiple exchanges. Real-time WebSocket streams.

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

---

Pulpo Loco provides a single `Exchange` trait that abstracts away the differences between crypto exchange APIs.
Write your trading logic once — run it on Binance, Bitget, Bybit, OKX, and more.

</div>

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      gateway-manager                         │
│          GatewayManager: register / query / merge            │
└────────┬──────────────────┬──────────────────┬──────────────┘
         │                  │                  │
┌────────▼────────┐ ┌──────▼────────┐ ┌───────▼─────────┐
│ gateway-binance  │ │ gateway-bitget │ │  gateway-bybit   │
│  REST + WS       │ │  REST + WS     │ │  REST + WS       │
└────────┬────────┘ └──────┬────────┘ └───────┬─────────┘
         │                  │                  │
┌────────▼──────────────────▼──────────────────▼─────────────┐
│                       gateway-core                          │
│       Exchange trait, types, errors, config                  │
└─────────────────────────────────────────────────────────────┘
```

| Crate | Description |
|---|---|
| `gateway-core` | `Exchange` trait, unified types (`Symbol`, `OrderBook`, `Trade`, `Candle`, `Ticker`), error handling |
| `gateway-binance` | Binance implementation — REST API + WebSocket streams |
| `gateway-bitget` | Bitget implementation — REST API + WebSocket streams |
| `gateway-bybit` | Bybit implementation — REST API + WebSocket streams |
| `gateway-manager` | Multi-exchange orchestrator — parallel queries, merged streams |

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
gateway-core    = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
gateway-binance = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
gateway-bitget  = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
gateway-bybit   = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
gateway-okx     = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
gateway-manager = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
tokio = { version = "1", features = ["full"] }
```

```rust
use gateway_binance::Binance;
use gateway_core::{Exchange, Symbol};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let binance = Binance::public();
    let btc = Symbol::new("BTC", "USDT");

    let ticker = binance.ticker(&btc).await?;
    println!("BTC/USDT: {}", ticker.last_price);

    Ok(())
}
```

## API Overview

The `Exchange` trait provides a unified interface for all exchanges:

### REST Methods

| Method | Signature | Description |
|---|---|---|
| `exchange_info` | `async fn() -> Result<ExchangeInfo>` | All trading pairs and metadata |
| `ticker` | `async fn(&Symbol) -> Result<Ticker>` | 24h ticker for one pair |
| `all_tickers` | `async fn() -> Result<Vec<Ticker>>` | 24h tickers for all pairs |
| `orderbook` | `async fn(&Symbol, depth: u16) -> Result<OrderBook>` | Order book snapshot |
| `trades` | `async fn(&Symbol, limit: u16) -> Result<Vec<Trade>>` | Recent trades |
| `candles` | `async fn(&Symbol, Interval, limit: u16) -> Result<Vec<Candle>>` | Historical candlesticks |

### WebSocket Streams

| Method | Signature | Description |
|---|---|---|
| `stream_trades` | `async fn(&Symbol) -> Result<BoxStream<Trade>>` | Real-time trades |
| `stream_orderbook` | `async fn(&Symbol) -> Result<BoxStream<OrderBook>>` | Order book updates |
| `stream_candles` | `async fn(&Symbol, Interval) -> Result<BoxStream<Candle>>` | Candle updates |
| `stream_trades_batch` | `async fn(&[Symbol]) -> Result<BoxStream<Trade>>` | Multi-symbol trades (single connection) |
| `stream_orderbooks_batch` | `async fn(&[Symbol]) -> Result<BoxStream<OrderBook>>` | Multi-symbol order books |

### GatewayManager

| Method | Description |
|---|---|
| `register(impl Exchange)` | Add an exchange |
| `get(ExchangeId) -> Option<Arc<dyn Exchange>>` | Get exchange by ID |
| `all_tickers_everywhere()` | Fetch tickers from all exchanges in parallel |
| `stream_trades_multi(&[(ExchangeId, Symbol)])` | Merged trade stream from multiple exchanges |

## Examples

### basic_rest (Binance)

Exchange info, ticker, order book, trades, and candles:

```bash
cargo run -p gateway-binance --example basic_rest
```

### basic_rest (Bitget)

Same API, different exchange — demonstrates the unified trait:

```bash
cargo run -p gateway-bitget --example basic_rest
```

### basic_rest (Bybit)

Same API, different exchange — demonstrates the unified trait:

```bash
cargo run -p gateway-bybit --example basic_rest
```

### stream_trades

WebSocket trade streaming — prints first 20 BTC/USDT trades:

```bash
cargo run -p gateway-binance --example stream_trades
```

### multi_exchange

Multi-exchange orchestration — parallel tickers, price comparison, merged streams:

```bash
cargo run -p gateway-manager --example multi_exchange
```

## Supported Exchanges

| Exchange | REST | WebSocket | Batch WS |
|---|---|---|---|
| Binance | yes | yes | yes (combined stream) |
| Bitget | yes | yes | yes (multi-topic) |
| Bybit | yes | yes | yes (multi-topic) |

## Project Structure

```
pulpo_loco/
├── Cargo.toml                          # Workspace root
├── README.md
└── crates/
    ├── gateway-core/                   # Shared types & traits
    │   └── src/
    │       ├── lib.rs
    │       ├── types.rs                # Symbol, OrderBook, Trade, Candle, Ticker, ...
    │       ├── traits.rs               # Exchange, ExchangeTrading traits
    │       ├── error.rs                # GatewayError
    │       ├── config.rs               # ExchangeConfig, RestConfig, WsConfig
    │       └── stream.rs               # BoxStream, StreamEvent, Subscription
    ├── gateway-binance/                # Binance implementation
    │   ├── src/
    │   │   ├── lib.rs                  # Binance struct + Exchange impl
    │   │   ├── rest.rs                 # REST client
    │   │   ├── ws.rs                   # WebSocket streams
    │   │   └── mapper.rs               # Symbol/interval conversion
    │   └── examples/
    │       ├── basic_rest.rs
    │       └── stream_trades.rs
    ├── gateway-bitget/                 # Bitget implementation
    │   ├── src/
    │   │   ├── lib.rs                  # Bitget struct + Exchange impl
    │   │   ├── rest.rs                 # REST client
    │   │   ├── ws.rs                   # WebSocket streams
    │   │   └── mapper.rs               # Symbol/interval conversion
    │   └── examples/
    │       └── basic_rest.rs
    ├── gateway-bybit/                  # Bybit implementation
    │   ├── src/
    │   │   ├── lib.rs                  # Bybit struct + Exchange impl
    │   │   ├── rest.rs                 # REST client
    │   │   ├── ws.rs                   # WebSocket streams
    │   │   └── mapper.rs               # Symbol/interval conversion
    │   └── examples/
    │       └── basic_rest.rs
    └── gateway-manager/                # Multi-exchange orchestrator
        ├── src/
        │   └── lib.rs                  # GatewayManager
        └── examples/
            └── multi_exchange.rs
```
