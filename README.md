<div align="center">

<img src="logo.png" alt="Pulpo Loco" width="200">

# Pulpo Loco

**Unified Rust gateway for cryptocurrency exchanges**

One trait. Multiple exchanges. Spot & Futures. Real-time WebSocket streams.

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

---

Pulpo Loco provides a unified `Exchange` trait that abstracts away the differences between crypto exchange APIs.
Write your trading logic once — run it on Binance, Bitget, Bybit, OKX, Gate.io, MEXC, KuCoin, Lighter, Asterdex, Hyperliquid, Phemex, Blofin, Toobit, Bitunix, XT.com, and more.

</div>

## Architecture

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                                              gateway-manager                                                                  │
│                                   GatewayManager: register / register_futures / query / merge                                                 │
└──┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬────────┘
   │          │          │          │          │          │          │          │          │          │          │          │          │
┌──▼───────┐┌─▼────────┐┌▼────────┐┌▼────────┐┌▼────────┐┌▼───────┐┌▼────────┐┌▼─────────┐┌▼─────────┐┌▼────────┐┌▼────────┐┌▼────────┐┌▼──────┐
│ binance  ││ bitget   ││ bybit   ││ okx     ││ gate    ││ mexc   ││ kucoin  ││ lighter  ││asterdex  ││hyperl.  ││ phemex  ││ blofin  ││toobit │
│ spot+fut ││ spot+fut ││ spot+fut ││ spot+fut ││ spot+fut││spot+fut││ spot    ││ futures  ││ futures  ││ futures ││ futures ││ futures ││futures│
│ REST+WS  ││ REST+WS  ││ REST+WS  ││ REST+WS  ││ REST+WS ││ REST+WS││ REST+WS ││ REST+WS  ││ REST+WS  ││ REST+WS ││ REST+WS ││ REST+WS ││REST+WS│
└──┬───────┘└─┬────────┘└┬────────┘└┬────────┘└┬────────┘└┬───────┘└┬────────┘└┬─────────┘└┬─────────┘└┬────────┘└┬────────┘└┬────────┘└┬──────┘
   │          │          │          │          │          │          │          │           │           │          │          │          │
   │  ┌───────────────────────────────────────────────────────────────────────────────────────────────────────────────┐       │          │
   │  │                                        + bitunix (futures) + xt (futures)                                     │       │          │
   │  └───────────────────────────────────────────────────────────────────────────────────────────────────────────────┘       │          │
   │          │          │          │          │          │          │          │           │           │          │          │          │
┌──▼──────────▼──────────▼──────────▼──────────▼──────────▼──────────▼──────────▼───────────▼───────────▼──────────▼──────────▼──────────▼──────┐
│                                                         gateway-core                                                                          │
│                              Exchange + FuturesExchange traits, types, errors, config                                                         │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

| Crate | Description |
|---|---|
| `gateway-core` | `Exchange` + `FuturesExchange` traits, unified types (`Symbol`, `OrderBook`, `Trade`, `Candle`, `Ticker`, `FundingRate`, `MarkPrice`, `OpenInterest`, `Liquidation`), error handling |
| `gateway-binance` | Binance Spot & Futures — REST + WebSocket |
| `gateway-bitget` | Bitget Spot & Futures — REST + WebSocket |
| `gateway-bybit` | Bybit Spot & Futures — REST + WebSocket |
| `gateway-okx` | OKX Spot & Futures — REST + WebSocket |
| `gateway-gate` | Gate.io Spot & Futures — REST + WebSocket |
| `gateway-mexc` | MEXC Spot & Futures — REST + WebSocket (protobuf) |
| `gateway-kucoin` | KuCoin Spot — REST + WebSocket |
| `gateway-lighter` | Lighter DEX Futures — REST + WebSocket |
| `gateway-asterdex` | AsterDEX Futures — REST + WebSocket |
| `gateway-hyperliquid` | Hyperliquid Futures — REST + WebSocket |
| `gateway-phemex` | Phemex Futures — REST + WebSocket |
| `gateway-blofin` | BloFin Futures — REST + WebSocket |
| `gateway-toobit` | Toobit Futures — REST + WebSocket |
| `gateway-bitunix` | Bitunix Futures — REST + WebSocket |
| `gateway-xt` | XT.com Futures — REST + WebSocket |
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
cargo run -p gateway-binance     --example futures_rest
cargo run -p gateway-bitget      --example futures_rest
cargo run -p gateway-bybit       --example futures_rest
cargo run -p gateway-okx         --example futures_rest
cargo run -p gateway-gate        --example futures_rest
cargo run -p gateway-lighter     --example futures_rest
cargo run -p gateway-asterdex    --example futures_rest
cargo run -p gateway-hyperliquid --example futures_rest
cargo run -p gateway-phemex      --example futures_rest
cargo run -p gateway-blofin      --example futures_rest
cargo run -p gateway-toobit      --example futures_rest
cargo run -p gateway-bitunix     --example futures_rest
cargo run -p gateway-xt          --example futures_rest
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

Futures orderbook, trades, mark price, liquidations:

```bash
cargo run -p gateway-bitget      --example stream_futures
cargo run -p gateway-gate        --example stream_futures
cargo run -p gateway-mexc        --example stream_futures
cargo run -p gateway-lighter     --example stream_trades
cargo run -p gateway-lighter     --example stream_orderbook
cargo run -p gateway-lighter     --example stream_mark_price
cargo run -p gateway-asterdex    --example stream_orderbook
cargo run -p gateway-asterdex    --example stream_trades
cargo run -p gateway-asterdex    --example stream_mark_price
cargo run -p gateway-hyperliquid --example stream_orderbook
cargo run -p gateway-hyperliquid --example stream_trades
cargo run -p gateway-phemex      --example stream_orderbook
cargo run -p gateway-phemex      --example stream_trades
cargo run -p gateway-blofin      --example stream_orderbook
cargo run -p gateway-blofin      --example stream_trades
cargo run -p gateway-toobit      --example stream_orderbook
cargo run -p gateway-toobit      --example stream_trades
cargo run -p gateway-bitunix     --example stream_orderbook
cargo run -p gateway-bitunix     --example stream_trades
cargo run -p gateway-xt          --example stream_orderbook
cargo run -p gateway-xt          --example stream_trades
```

### Batch WebSocket (Futures)

Subscribe to all symbols at once (multi-connection sharding):

```bash
cargo run -p gateway-hyperliquid --example stream_orderbook_all
cargo run -p gateway-phemex      --example stream_orderbook_all
cargo run -p gateway-blofin      --example stream_orderbook_all
cargo run -p gateway-toobit      --example stream_orderbook_all
cargo run -p gateway-bitunix     --example stream_orderbook_all
cargo run -p gateway-xt          --example stream_orderbook_all
cargo run -p gateway-mexc        --example stream_futures_batch
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
| MEXC | yes | yes | yes | yes | yes (multi-topic) |
| KuCoin | yes | — | yes | yes | yes (multi-topic) |
| Lighter | — | yes | yes | yes | yes (chunked) |
| AsterDEX | — | yes | yes | yes | yes (chunked) |
| Hyperliquid | — | yes | yes | yes | yes (chunked) |
| Phemex | — | yes | yes | yes | yes (chunked) |
| BloFin | — | yes | yes | yes | yes (chunked) |
| Toobit | — | yes | yes | yes | yes (chunked) |
| Bitunix | — | yes | yes | yes | yes (chunked) |
| XT.com | — | yes | yes | yes | yes (chunked) |

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
    │
    │   ── Spot & Futures ──────────────────────────────────────────────────
    │
    ├── gateway-binance/                # Binance (Spot & Futures)
    │   ├── src/{lib,spot/,futures/}    # BinanceSpot, BinanceFutures
    │   └── examples/                   # basic_rest, futures_rest, stream_trades
    ├── gateway-bitget/                 # Bitget (Spot & Futures)
    │   ├── src/{lib,spot/,futures/}    # BitgetSpot, BitgetFutures
    │   └── examples/                   # basic_rest, futures_rest, stream_trades, stream_futures
    ├── gateway-bybit/                  # Bybit (Spot & Futures)
    │   ├── src/{lib,spot/,futures/}    # BybitSpot, BybitFutures
    │   └── examples/                   # basic_rest, futures_rest, stream_trades
    ├── gateway-okx/                    # OKX (Spot & Futures)
    │   ├── src/{lib,spot/,futures/}    # OkxSpot, OkxFutures
    │   └── examples/                   # basic_rest, futures_rest, stream_trades
    ├── gateway-gate/                   # Gate.io (Spot & Futures)
    │   ├── src/{lib,spot/,futures/}    # GateSpot, GateFutures
    │   └── examples/                   # basic_rest, futures_rest, stream_trades, stream_futures
    ├── gateway-mexc/                   # MEXC (Spot & Futures)
    │   ├── src/{lib,spot/,futures/}    # MexcSpot, MexcFutures
    │   └── examples/                   # basic_rest, stream_trades, stream_futures, stream_futures_batch
    │
    │   ── Spot Only ───────────────────────────────────────────────────────
    │
    ├── gateway-kucoin/                 # KuCoin (Spot only)
    │   ├── src/{lib,spot/}             # KucoinSpot
    │   └── examples/                   # stream_trades, ws_debug
    │
    │   ── Futures Only (Perpetuals) ───────────────────────────────────────
    │
    ├── gateway-lighter/                # Lighter DEX
    │   ├── src/{lib,futures/}          # LighterFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_mark_price
    ├── gateway-asterdex/               # AsterDEX
    │   ├── src/{lib,futures/}          # AsterdexFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_mark_price, stream_liquidations
    ├── gateway-hyperliquid/            # Hyperliquid
    │   ├── src/{lib,futures/}          # HyperliquidFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_orderbook_all, stream_mark_price
    ├── gateway-phemex/                 # Phemex
    │   ├── src/{lib,futures/}          # PhemexFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_orderbook_all
    ├── gateway-blofin/                 # BloFin
    │   ├── src/{lib,futures/}          # BlofinFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_orderbook_all
    ├── gateway-toobit/                 # Toobit
    │   ├── src/{lib,futures/}          # ToobitFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_orderbook_all
    ├── gateway-bitunix/                # Bitunix
    │   ├── src/{lib,futures/}          # BitunixFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_orderbook_all
    ├── gateway-xt/                     # XT.com
    │   ├── src/{lib,futures/}          # XtFutures
    │   └── examples/                   # futures_rest, stream_trades, stream_orderbook, stream_orderbook_all
    │
    │   ── Orchestrator ────────────────────────────────────────────────────
    │
    └── gateway-manager/                # Multi-exchange orchestrator
        ├── src/lib.rs                  # GatewayManager
        └── examples/                   # multi_exchange
```
