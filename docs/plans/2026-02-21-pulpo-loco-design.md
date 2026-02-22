# pulpo_loco - Crypto Exchange Gateway

## Architecture

Rust workspace with 4 crates:

- **gateway-core** — types, traits, errors (zero exchange dependencies)
- **gateway-binance** — Binance REST + WebSocket (spot)
- **gateway-bybit** — Bybit V5 REST + WebSocket (unified)
- **gateway-manager** — exchange multiplexer

## Exchange Trait

Every exchange implements: `exchange_info`, `orderbook`, `trades`, `candles`, `ticker`, `all_tickers` (REST) + `stream_orderbook`, `stream_trades`, `stream_candles`, `stream_*_batch` (WS).

## Key Types

`Symbol`, `OrderBook`, `Trade`, `Candle`, `Ticker`, `Level`, `SymbolInfo`, `ExchangeInfo`, `StreamEvent`

## Dependencies

- `tokio` + `tokio-tungstenite` for async + WS
- `reqwest` for REST
- `rust_decimal` for precise decimal math
- `serde` + `serde_json` for serialization
- `tracing` for logging
