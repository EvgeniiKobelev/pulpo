use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Exchange Info
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceExchangeInfoRaw {
    pub symbols: Vec<BinanceSymbolRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BinanceSymbolRaw {
    pub symbol: String,
    pub status: String,
    #[serde(rename = "baseAsset")]
    pub base_asset: String,
    #[serde(rename = "quoteAsset")]
    pub quote_asset: String,
    #[serde(rename = "baseAssetPrecision")]
    pub base_asset_precision: u8,
    #[serde(rename = "quoteAssetPrecision")]
    pub quote_asset_precision: u8,
    #[serde(default)]
    pub filters: Vec<serde_json::Value>,
}

impl BinanceExchangeInfoRaw {
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
                        "NOTIONAL" | "MIN_NOTIONAL" => {
                            min_notional = f
                                .get("minNotional")
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
                    base_precision: s.base_asset_precision,
                    quote_precision: s.quote_asset_precision,
                    min_qty,
                    min_notional,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::Binance,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// REST OrderBook
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceOrderBookRaw {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

impl BinanceOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::Binance,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: 0,
            sequence: Some(self.last_update_id),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Trade
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceTradeRaw {
    pub id: u64,
    pub price: String,
    pub qty: String,
    pub time: u64,
    #[serde(rename = "isBuyerMaker")]
    pub is_buyer_maker: bool,
}

impl BinanceTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        Trade {
            exchange: ExchangeId::Binance,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker {
                Side::Sell
            } else {
                Side::Buy
            },
            timestamp_ms: self.time,
            trade_id: Some(self.id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Ticker
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "bidPrice")]
    pub bid_price: String,
    #[serde(rename = "askPrice")]
    pub ask_price: String,
    pub volume: String,
    #[serde(rename = "priceChangePercent")]
    pub price_change_percent: String,
    #[serde(rename = "closeTime")]
    pub close_time: u64,
}

impl BinanceTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::Binance,
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
// WS Depth
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
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

// ---------------------------------------------------------------------------
// WS Trade
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
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

impl BinanceWsTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Trade {
            exchange: ExchangeId::Binance,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker {
                Side::Sell
            } else {
                Side::Buy
            },
            timestamp_ms: self.trade_time,
            trade_id: Some(self.trade_id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// WS Kline
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceWsKlineMsg {
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    pub k: BinanceWsKlineRaw,
}

#[derive(Debug, Deserialize)]
pub struct BinanceWsKlineRaw {
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

impl BinanceWsKlineMsg {
    pub fn into_candle(self) -> Candle {
        let symbol = binance_symbol_to_unified(&self.symbol);
        Candle {
            exchange: ExchangeId::Binance,
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

// ---------------------------------------------------------------------------
// Combined stream wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BinanceCombinedStream {
    pub stream: String,
    pub data: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Helper functions
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

/// Convert a unified Symbol to a Binance raw symbol string (e.g. "BTCUSDT").
pub fn unified_to_binance(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Known quote assets in priority order (longest first to avoid partial matches).
const KNOWN_QUOTES: &[&str] = &[
    "USDT", "BUSD", "USDC", "TUSD", "FDUSD", "DUSDT",
    "BTC", "ETH", "BNB", "EUR", "GBP", "TRY", "BRL",
    "ARS", "BIDR", "DAI", "IDRT", "NGN", "PLN", "RON",
    "RUB", "UAH", "ZAR", "VAI", "UST", "AUD", "JPY",
];

/// Convert a Binance raw symbol string (e.g. "BTCUSDT") to a unified Symbol.
pub fn binance_symbol_to_unified(raw: &str) -> Symbol {
    let upper = raw.to_uppercase();
    for quote in KNOWN_QUOTES {
        if upper.ends_with(quote) {
            let base = &upper[..upper.len() - quote.len()];
            if !base.is_empty() {
                return Symbol::new(base, *quote);
            }
        }
    }
    // Fallback: treat the whole string as base with empty quote
    Symbol::new(&upper, "")
}

/// Map a unified Interval to the Binance interval string.
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

// ---------------------------------------------------------------------------
// REST Kline parsing
// ---------------------------------------------------------------------------

/// Parse a single Binance kline array row into a Candle.
///
/// Binance returns klines as arrays:
/// `[openTime, open, high, low, close, volume, closeTime, ...]`
pub fn parse_kline_row(row: &[serde_json::Value], symbol: Symbol) -> Option<Candle> {
    Some(Candle {
        exchange: ExchangeId::Binance,
        symbol,
        open: Decimal::from_str(row.get(1)?.as_str()?).ok()?,
        high: Decimal::from_str(row.get(2)?.as_str()?).ok()?,
        low: Decimal::from_str(row.get(3)?.as_str()?).ok()?,
        close: Decimal::from_str(row.get(4)?.as_str()?).ok()?,
        volume: Decimal::from_str(row.get(5)?.as_str()?).ok()?,
        open_time_ms: row.get(0)?.as_u64()?,
        close_time_ms: row.get(6)?.as_u64()?,
        is_closed: true,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_unified_to_binance() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_binance(&sym), "BTCUSDT");

        let sym2 = Symbol::new("ETH", "BTC");
        assert_eq!(unified_to_binance(&sym2), "ETHBTC");
    }

    #[test]
    fn test_binance_symbol_to_unified() {
        let sym = binance_symbol_to_unified("BTCUSDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = binance_symbol_to_unified("ETHBTC");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "BTC");

        let sym3 = binance_symbol_to_unified("SOLUSDC");
        assert_eq!(sym3.base, "SOL");
        assert_eq!(sym3.quote, "USDC");

        let sym4 = binance_symbol_to_unified("BNBFDUSD");
        assert_eq!(sym4.base, "BNB");
        assert_eq!(sym4.quote, "FDUSD");
    }

    #[test]
    fn test_parse_levels() {
        let raw = vec![
            ["100.50".to_string(), "1.5".to_string()],
            ["99.00".to_string(), "2.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(100.50));
        assert_eq!(levels[0].qty, dec!(1.5));
        assert_eq!(levels[1].price, dec!(99.00));
        assert_eq!(levels[1].qty, dec!(2.0));
    }

    #[test]
    fn test_parse_levels_skips_invalid() {
        let raw = vec![
            ["bad".to_string(), "1.0".to_string()],
            ["50.00".to_string(), "3.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, dec!(50.00));
    }

    #[test]
    fn test_interval_to_binance() {
        assert_eq!(interval_to_binance(Interval::S1), "1s");
        assert_eq!(interval_to_binance(Interval::M1), "1m");
        assert_eq!(interval_to_binance(Interval::M3), "3m");
        assert_eq!(interval_to_binance(Interval::M5), "5m");
        assert_eq!(interval_to_binance(Interval::M15), "15m");
        assert_eq!(interval_to_binance(Interval::M30), "30m");
        assert_eq!(interval_to_binance(Interval::H1), "1h");
        assert_eq!(interval_to_binance(Interval::H4), "4h");
        assert_eq!(interval_to_binance(Interval::D1), "1d");
        assert_eq!(interval_to_binance(Interval::W1), "1w");
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw = BinanceOrderBookRaw {
            last_update_id: 42,
            bids: vec![
                ["50000.00".to_string(), "1.0".to_string()],
                ["49999.00".to_string(), "0.5".to_string()],
            ],
            asks: vec![
                ["50001.00".to_string(), "2.0".to_string()],
            ],
        };
        let symbol = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(symbol);

        assert_eq!(ob.exchange, ExchangeId::Binance);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(42));
    }

    #[test]
    fn test_ticker_conversion() {
        let raw: BinanceTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPrice": "50000.00",
                "bidPrice": "49999.00",
                "askPrice": "50001.00",
                "volume": "12345.678",
                "priceChangePercent": "2.5",
                "closeTime": 1700000000000
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::Binance);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(2.5)));
        assert_eq!(ticker.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_conversion() {
        let raw = BinanceTradeRaw {
            id: 999,
            price: "50000.50".to_string(),
            qty: "0.01".to_string(),
            time: 1700000000000,
            is_buyer_maker: true,
        };
        let trade = raw.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::Binance);
        assert_eq!(trade.price, dec!(50000.50));
        assert_eq!(trade.qty, dec!(0.01));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.trade_id, Some("999".to_string()));

        let raw2 = BinanceTradeRaw {
            id: 1000,
            price: "50001.00".to_string(),
            qty: "0.02".to_string(),
            time: 1700000000001,
            is_buyer_maker: false,
        };
        let trade2 = raw2.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade2.side, Side::Buy);
    }

    #[test]
    fn test_ws_depth_conversion() {
        let raw: BinanceWsDepthRaw = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "b": [["50000.00", "1.0"]],
                "a": [["50001.00", "2.0"]],
                "E": 1700000000000,
                "u": 100
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.timestamp_ms, 1700000000000);
        assert_eq!(ob.sequence, Some(100));
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw: BinanceWsTradeRaw = serde_json::from_str(
            r#"{
                "s": "ETHUSDT",
                "p": "2000.00",
                "q": "0.5",
                "T": 1700000000000,
                "t": 12345,
                "m": false
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.00));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("12345".to_string()));
    }

    #[test]
    fn test_ws_kline_conversion() {
        let raw: BinanceWsKlineMsg = serde_json::from_str(
            r#"{
                "E": 1700000000000,
                "s": "BTCUSDT",
                "k": {
                    "t": 1700000000000,
                    "T": 1700000060000,
                    "o": "50000.00",
                    "c": "50100.00",
                    "h": "50200.00",
                    "l": "49900.00",
                    "v": "100.5",
                    "x": true
                }
            }"#,
        )
        .unwrap();

        let candle = raw.into_candle();
        assert_eq!(candle.exchange, ExchangeId::Binance);
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert!(candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row() {
        let row: Vec<serde_json::Value> = serde_json::from_str(
            r#"[1700000000000, "50000.00", "50200.00", "49900.00", "50100.00", "100.5", 1700000060000, "0", 0, "0", "0", "0"]"#,
        )
        .unwrap();

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::Binance);
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000);
        assert!(candle.is_closed);
    }

    #[test]
    fn test_exchange_info_conversion() {
        let raw: BinanceExchangeInfoRaw = serde_json::from_str(
            r#"{
                "symbols": [{
                    "symbol": "BTCUSDT",
                    "status": "TRADING",
                    "baseAsset": "BTC",
                    "quoteAsset": "USDT",
                    "baseAssetPrecision": 8,
                    "quoteAssetPrecision": 8,
                    "filters": [
                        {"filterType": "LOT_SIZE", "minQty": "0.00001000"},
                        {"filterType": "PRICE_FILTER", "tickSize": "0.01000000"},
                        {"filterType": "NOTIONAL", "minNotional": "10.00000000"}
                    ]
                }, {
                    "symbol": "ETHBTC",
                    "status": "HALT",
                    "baseAsset": "ETH",
                    "quoteAsset": "BTC",
                    "baseAssetPrecision": 8,
                    "quoteAssetPrecision": 8,
                    "filters": []
                }]
            }"#,
        )
        .unwrap();

        let info = raw.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::Binance);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTCUSDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 8);
        assert_eq!(btc.min_qty, Some(dec!(0.00001000)));
        assert_eq!(btc.tick_size, Some(dec!(0.01000000)));
        assert_eq!(btc.min_notional, Some(dec!(10.00000000)));

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::Halted);
        assert_eq!(eth.min_qty, None);
    }

    #[test]
    fn test_exchange_info_status_mapping() {
        let make_raw = |status: &str| BinanceExchangeInfoRaw {
            symbols: vec![BinanceSymbolRaw {
                symbol: "XYZUSDT".to_string(),
                status: status.to_string(),
                base_asset: "XYZ".to_string(),
                quote_asset: "USDT".to_string(),
                base_asset_precision: 8,
                quote_asset_precision: 8,
                filters: vec![],
            }],
        };

        assert_eq!(
            make_raw("TRADING").into_exchange_info().symbols[0].status,
            SymbolStatus::Trading
        );
        assert_eq!(
            make_raw("HALT").into_exchange_info().symbols[0].status,
            SymbolStatus::Halted
        );
        assert_eq!(
            make_raw("PRE_TRADING").into_exchange_info().symbols[0].status,
            SymbolStatus::PreTrading
        );
        assert_eq!(
            make_raw("BREAK").into_exchange_info().symbols[0].status,
            SymbolStatus::Unknown
        );
    }

    #[test]
    fn test_combined_stream_deserialization() {
        let raw: BinanceCombinedStream = serde_json::from_str(
            r#"{"stream": "btcusdt@depth", "data": {"s": "BTCUSDT"}}"#,
        )
        .unwrap();
        assert_eq!(raw.stream, "btcusdt@depth");
        assert!(raw.data.is_object());
    }
}
