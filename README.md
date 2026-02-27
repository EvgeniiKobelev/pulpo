<div align="center">

<img src="logo.png" alt="Pulpo Loco" width="200">

# Pulpo Loco

**Unified Rust gateway for cryptocurrency exchanges**

One trait. Multiple exchanges. Spot & Futures. Real-time WebSocket streams.

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

---

Pulpo Loco provides a unified `Exchange` trait that abstracts away the differences between crypto exchange APIs.
Write your trading logic once — run it on Binance, Bitget, Bybit, OKX, Gate.io, MEXC, KuCoin, and more.

</div>

## Architecture

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                        gateway-manager                                            │
│             GatewayManager: register / register_futures / query / merge                            │
└──┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────────────────────────┘
   │          │          │          │          │          │          │
┌──▼───────┐┌─▼────────┐┌▼────────┐┌▼────────┐┌▼────────┐┌▼───────┐┌▼────────┐
│ binance  ││ bitget   ││ bybit   ││ okx     ││ gate    ││ mexc   ││ kucoin  │
│ spot+fut ││ spot+fut ││ spot+fut ││ spot+fut ││ spot+fut││ spot   ││ spot    │
│ REST+WS  ││ REST+WS  ││ REST+WS  ││ REST+WS  ││ REST+WS ││ REST+WS││ REST+WS │
└──┬───────┘└─┬────────┘└┬────────┘└┬────────┘└┬────────┘└┬───────┘└┬────────┘
   │          │          │          │          │          │          │
┌──▼──────────▼──────────▼──────────▼──────────▼──────────▼──────────▼──────────┐
│                                   gateway-core                                 │
│         Exchange + FuturesExchange traits, types, errors, config               │
└──────────────────────────────────────────────────────────────────────────────────┘
```

| Crate | Description |
|---|---|
| `gateway-core` | `Exchange` + `FuturesExchange` traits, unified types (`Symbol`, `OrderBook`, `Trade`, `Candle`, `Ticker`, `FundingRate`, `MarkPrice`, `OpenInterest`, `Liquidation`), error handling |
| `gateway-binance` | Binance Spot & Futures — REST + WebSocket |
| `gateway-bitget` | Bitget Spot & Futures — REST + WebSocket |
| `gateway-bybit` | Bybit Spot & Futures — REST + WebSocket |
| `gateway-okx` | OKX Spot & Futures — REST + WebSocket |
| `gateway-gate` | Gate.io Spot & Futures — REST + WebSocket |
| `gateway-mexc` | MEXC Spot — REST + WebSocket (protobuf) |
| `gateway-kucoin` | KuCoin Spot — REST + WebSocket |
| `gateway-manager` | Multi-exchange orchestrator — parallel queries, merged streams, futures aggregation |

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
gateway-core    = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
gateway-binance = { git = "https://github.com/EvgeniiKobelev/pulpo.git" }
tokio = { version = "1", features = ["full"] }
```

### Spot

```rust
use gateway_binance::BinanceSpot;
use gateway_core::{Exchange, Symbol};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let binance = BinanceSpot::public();
    let btc = Symbol::new("BTC", "USDT");

    let ticker = binance.ticker(&btc).await?;
    println!("BTC/USDT: {}", ticker.last_price);

    Ok(())
}
```

### Futures

```rust
use gateway_binance::BinanceFutures;
use gateway_core::{Exchange, FuturesExchange, Symbol};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let futures = BinanceFutures::public();
    let btc = Symbol::new("BTC", "USDT");

    let funding = futures.funding_rate(&btc).await?;
    println!("Funding rate: {}", funding.rate);

    let mark = futures.mark_price(&btc).await?;
    println!("Mark price: {}", mark.mark_price);

    Ok(())
}
```

## API Overview

### Exchange trait

The base `Exchange` trait provides a unified interface for all spot and futures markets:

#### REST Methods

| Method | Signature | Description |
|---|---|---|
| `exchange_info` | `async fn() -> Result<ExchangeInfo>` | All trading pairs and metadata |
| `ticker` | `async fn(&Symbol) -> Result<Ticker>` | 24h ticker for one pair |
| `all_tickers` | `async fn() -> Result<Vec<Ticker>>` | 24h tickers for all pairs |
| `orderbook` | `async fn(&Symbol, depth: u16) -> Result<OrderBook>` | Order book snapshot |
| `trades` | `async fn(&Symbol, limit: u16) -> Result<Vec<Trade>>` | Recent trades |
| `candles` | `async fn(&Symbol, Interval, limit: u16) -> Result<Vec<Candle>>` | Historical candlesticks |

#### WebSocket Streams

| Method | Signature | Description |
|---|---|---|
| `stream_trades` | `async fn(&Symbol) -> Result<BoxStream<Trade>>` | Real-time trades |
| `stream_orderbook` | `async fn(&Symbol) -> Result<BoxStream<OrderBook>>` | Order book updates |
| `stream_candles` | `async fn(&Symbol, Interval) -> Result<BoxStream<Candle>>` | Candle updates |
| `stream_trades_batch` | `async fn(&[Symbol]) -> Result<BoxStream<Trade>>` | Multi-symbol trades (single connection) |
| `stream_orderbooks_batch` | `async fn(&[Symbol]) -> Result<BoxStream<OrderBook>>` | Multi-symbol order books |

### FuturesExchange trait

Extends `Exchange` with futures-specific data:

| Method | Signature | Description |
|---|---|---|
| `funding_rate` | `async fn(&Symbol) -> Result<FundingRate>` | Current funding rate |
| `mark_price` | `async fn(&Symbol) -> Result<MarkPrice>` | Mark & index price |
| `open_interest` | `async fn(&Symbol) -> Result<OpenInterest>` | Open interest |
| `liquidations` | `async fn(&Symbol, limit: u16) -> Result<Vec<Liquidation>>` | Recent liquidations |
| `stream_mark_price` | `async fn(&Symbol) -> Result<BoxStream<MarkPrice>>` | Real-time mark price |
| `stream_liquidations` | `async fn(&Symbol) -> Result<BoxStream<Liquidation>>` | Real-time liquidations |

### GatewayManager

| Method | Description |
|---|---|
| `register(impl Exchange)` | Add a spot exchange |
| `register_futures(impl FuturesExchange)` | Add a futures exchange |
| `get(ExchangeId)` | Get exchange by ID |
| `get_futures(ExchangeId)` | Get futures exchange by ID |
| `all()` | All registered exchanges |
| `all_tickers_everywhere()` | Fetch tickers from all exchanges in parallel |
| `stream_trades_multi(&[(ExchangeId, Symbol)])` | Merged trade stream from multiple exchanges |
| `all_funding_rates(&Symbol)` | Funding rates from all futures exchanges in parallel |
| `stream_liquidations_multi(&[(ExchangeId, Symbol)])` | Merged liquidation stream from multiple futures exchanges |

## Examples

### Spot REST

Exchange info, ticker, order book, trades, and candles:

```bash
cargo run -p gateway-binance --example basic_rest
cargo run -p gateway-bitget  --example basic_rest
cargo run -p gateway-bybit   --example basic_rest
cargo run -p gateway-okx     --example basic_rest
cargo run -p gateway-gate    --example basic_rest
cargo run -p gateway-mexc    --example basic_rest
```

### Futures REST

Funding rate, mark price, open interest, order book:

```bash
cargo run -p gateway-binance --example futures_rest
cargo run -p gateway-bitget  --example futures_rest
cargo run -p gateway-bybit   --example futures_rest
cargo run -p gateway-okx     --example futures_rest
cargo run -p gateway-gate    --example futures_rest
```

### WebSocket Streams (Spot)

Real-time trade streaming:

```bash
cargo run -p gateway-binance --example stream_trades
cargo run -p gateway-bitget  --example stream_trades
cargo run -p gateway-bybit   --example stream_trades
cargo run -p gateway-okx     --example stream_trades
cargo run -p gateway-gate    --example stream_trades
cargo run -p gateway-mexc    --example stream_trades
cargo run -p gateway-kucoin  --example stream_trades
```

### WebSocket Streams (Futures)

Futures orderbook, trades, candles, mark price, liquidations:

```bash
cargo run -p gateway-bitget --example stream_futures
cargo run -p gateway-gate   --example stream_futures
```

### Multi-exchange

Parallel tickers, funding rates, merged streams across exchanges:

```bash
cargo run -p gateway-manager --example multi_exchange
```

## Supported Exchanges

| Exchange | Spot | Futures | REST | WebSocket | Batch WS |
|---|---|---|---|---|---|
| Binance | yes | yes | yes | yes | yes (combined stream) |
| Bitget | yes | yes | yes | yes | yes (multi-topic) |
| Bybit | yes | yes | yes | yes | yes (multi-topic) |
| OKX | yes | yes | yes | yes | yes (multi-topic) |
| Gate.io | yes | yes | yes | yes | yes (multi-topic) |
| MEXC | yes | — | yes | yes | yes (multi-topic) |
| KuCoin | yes | — | yes | yes | yes (multi-topic) |

## Project Structure

```
pulpo_loco/
├── Cargo.toml                          # Workspace root
├── README.md
└── crates/
    ├── gateway-core/                   # Shared types & traits
    │   └── src/
    │       ├── lib.rs
    │       ├── types.rs                # Symbol, OrderBook, Trade, Candle, Ticker, FundingRate, ...
    │       ├── traits.rs               # Exchange, ExchangeTrading, FuturesExchange traits
    │       ├── error.rs                # GatewayError
    │       ├── config.rs               # ExchangeConfig, RestConfig, WsConfig
    │       └── stream.rs               # BoxStream, StreamEvent, Subscription
    ├── gateway-binance/                # Binance (Spot & Futures)
    │   ├── src/
    │   │   ├── lib.rs                  # BinanceSpot, BinanceFutures
    │   │   ├── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs
    │   │   └── futures/                # Futures: mod.rs, rest.rs, ws.rs, mapper.rs
    │   └── examples/
    │       ├── basic_rest.rs
    │       ├── futures_rest.rs
    │       └── stream_trades.rs
    ├── gateway-bitget/                 # Bitget (Spot & Futures)
    │   ├── src/
    │   │   ├── lib.rs                  # BitgetSpot, BitgetFutures
    │   │   ├── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs
    │   │   └── futures/                # Futures: mod.rs, rest.rs, ws.rs, mapper.rs
    │   └── examples/
    │       ├── basic_rest.rs
    │       ├── futures_rest.rs
    │       ├── stream_trades.rs
    │       └── stream_futures.rs
    ├── gateway-bybit/                  # Bybit (Spot & Futures)
    │   ├── src/
    │   │   ├── lib.rs                  # BybitSpot, BybitFutures
    │   │   ├── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs
    │   │   └── futures/                # Futures: mod.rs, rest.rs, ws.rs, mapper.rs
    │   └── examples/
    │       ├── basic_rest.rs
    │       ├── futures_rest.rs
    │       └── stream_trades.rs
    ├── gateway-okx/                    # OKX (Spot & Futures)
    │   ├── src/
    │   │   ├── lib.rs                  # OkxSpot, OkxFutures
    │   │   ├── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs
    │   │   └── futures/                # Futures: mod.rs, rest.rs, ws.rs, mapper.rs
    │   └── examples/
    │       ├── basic_rest.rs
    │       ├── futures_rest.rs
    │       └── stream_trades.rs
    ├── gateway-gate/                   # Gate.io (Spot & Futures)
    │   ├── src/
    │   │   ├── lib.rs                  # GateSpot, GateFutures
    │   │   ├── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs
    │   │   └── futures/                # Futures: mod.rs, rest.rs, ws.rs, mapper.rs
    │   └── examples/
    │       ├── basic_rest.rs
    │       ├── futures_rest.rs
    │       ├── stream_trades.rs
    │       └── stream_futures.rs
    ├── gateway-mexc/                   # MEXC (Spot only)
    │   ├── src/
    │   │   ├── lib.rs                  # MexcSpot
    │   │   └── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs, proto.rs
    │   └── examples/
    │       ├── basic_rest.rs
    │       ├── stream_trades.rs
    │       └── ws_debug.rs
    ├── gateway-kucoin/                 # KuCoin (Spot only)
    │   ├── src/
    │   │   ├── lib.rs                  # KucoinSpot
    │   │   └── spot/                   # Spot: mod.rs, rest.rs, ws.rs, mapper.rs
    │   └── examples/
    │       ├── stream_trades.rs
    │       └── ws_debug.rs
    └── gateway-manager/                # Multi-exchange orchestrator
        ├── src/
        │   └── lib.rs                  # GatewayManager
        └── examples/
            └── multi_exchange.rs
```
