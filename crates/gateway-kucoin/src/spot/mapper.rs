use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// KuCoin API wrapper — all responses have { "code": "200000", "data": ... }
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct KucoinResponse<T> {
    pub code: String,
    pub data: T,
}

// ---------------------------------------------------------------------------
// Symbols (GET /api/v1/symbols)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinSymbolRaw {
    pub symbol: String,
    #[serde(default)]
    pub name: Option<String>,
    pub base_currency: String,
    pub quote_currency: String,
    #[serde(default)]
    pub base_min_size: Option<String>,
    #[serde(default)]
    pub quote_min_size: Option<String>,
    #[serde(default)]
    pub base_increment: Option<String>,
    #[serde(default)]
    pub quote_increment: Option<String>,
    #[serde(default)]
    pub price_increment: Option<String>,
    #[serde(default)]
    pub enable_trading: Option<bool>,
}

pub fn symbols_to_exchange_info(symbols: Vec<KucoinSymbolRaw>) -> ExchangeInfo {
    let list = symbols
        .into_iter()
        .map(|raw| {
            let status = if raw.enable_trading.unwrap_or(false) {
                SymbolStatus::Trading
            } else {
                SymbolStatus::Halted
            };

            let base_precision = raw
                .base_increment
                .as_deref()
                .map(decimal_precision)
                .unwrap_or(0);
            let quote_precision = raw
                .quote_increment
                .as_deref()
                .map(decimal_precision)
                .unwrap_or(0);
            let min_qty = raw
                .base_min_size
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok());
            let min_notional = raw
                .quote_min_size
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok());
            let tick_size = raw
                .price_increment
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok());

            SymbolInfo {
                symbol: Symbol::new(&raw.base_currency, &raw.quote_currency),
                raw_symbol: raw.symbol,
                status,
                base_precision,
                quote_precision,
                min_qty,
                min_notional,
                tick_size,
            }
        })
        .collect();

    ExchangeInfo {
        exchange: ExchangeId::Kucoin,
        symbols: list,
    }
}

/// Count decimal places from an increment string like "0.00000001" -> 8
fn decimal_precision(s: &str) -> u8 {
    match s.find('.') {
        Some(pos) => {
            let frac = &s[pos + 1..];
            frac.len() as u8
        }
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /api/v1/market/orderbook/level2_{depth})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct KucoinOrderBookRaw {
    #[serde(default)]
    pub time: Option<u64>,
    #[serde(default)]
    pub sequence: Option<String>,
    pub bids: Vec<Vec<String>>,
    pub asks: Vec<Vec<String>>,
}

impl KucoinOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        let seq = self
            .sequence
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok());
        OrderBook {
            exchange: ExchangeId::Kucoin,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.time.unwrap_or(0),
            sequence: seq,
        }
    }
}

// ---------------------------------------------------------------------------
// Trades (GET /api/v1/market/histories)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct KucoinTradeRaw {
    pub sequence: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub time: u64, // nanoseconds
}

impl KucoinTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        Trade {
            exchange: ExchangeId::Kucoin,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side: parse_side(&self.side),
            timestamp_ms: self.time / 1_000_000, // ns -> ms
            trade_id: Some(self.sequence),
        }
    }
}

// ---------------------------------------------------------------------------
// Tickers
// ---------------------------------------------------------------------------

/// Single ticker (GET /api/v1/market/stats)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinTickerStatsRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(default)]
    pub buy: Option<String>,
    #[serde(default)]
    pub sell: Option<String>,
    #[serde(default)]
    pub vol: Option<String>,
    #[serde(default)]
    pub change_rate: Option<String>,
    #[serde(default)]
    pub time: Option<u64>,
}

impl KucoinTickerStatsRaw {
    pub fn into_ticker(self, symbol: Symbol) -> Ticker {
        let change_pct = self
            .change_rate
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .map(|d| d * Decimal::from(100)); // KuCoin returns 0.01 for 1%
        Ticker {
            exchange: ExchangeId::Kucoin,
            symbol,
            last_price: parse_decimal_opt(self.last.as_deref()),
            bid: self.buy.as_deref().and_then(|s| Decimal::from_str(s).ok()),
            ask: self.sell.as_deref().and_then(|s| Decimal::from_str(s).ok()),
            volume_24h: parse_decimal_opt(self.vol.as_deref()),
            price_change_pct_24h: change_pct,
            timestamp_ms: self.time.unwrap_or(0),
        }
    }
}

/// All tickers (GET /api/v1/market/allTickers)
#[derive(Debug, Deserialize)]
pub struct KucoinAllTickersResponse {
    #[serde(default)]
    pub time: Option<u64>,
    pub ticker: Vec<KucoinAllTickerRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinAllTickerRaw {
    pub symbol: String,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(default)]
    pub buy: Option<String>,
    #[serde(default)]
    pub sell: Option<String>,
    #[serde(default)]
    pub vol: Option<String>,
    #[serde(default)]
    pub change_rate: Option<String>,
}

impl KucoinAllTickerRaw {
    pub fn into_ticker(self, ts: u64) -> Ticker {
        let symbol = kucoin_pair_to_unified(&self.symbol);
        let change_pct = self
            .change_rate
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .map(|d| d * Decimal::from(100));
        Ticker {
            exchange: ExchangeId::Kucoin,
            symbol,
            last_price: parse_decimal_opt(self.last.as_deref()),
            bid: self.buy.as_deref().and_then(|s| Decimal::from_str(s).ok()),
            ask: self.sell.as_deref().and_then(|s| Decimal::from_str(s).ok()),
            volume_24h: parse_decimal_opt(self.vol.as_deref()),
            price_change_pct_24h: change_pct,
            timestamp_ms: ts,
        }
    }
}

// ---------------------------------------------------------------------------
// Klines (GET /api/v1/market/candles)
// ---------------------------------------------------------------------------

/// Parse a KuCoin kline row.
///
/// Row format: [time, open, close, high, low, volume, turnover]
pub fn parse_kline_row(row: &[String], symbol: Symbol) -> Option<Candle> {
    if row.len() < 7 {
        return None;
    }
    let open_time_secs: u64 = row[0].parse().ok()?;
    let open_time_ms = open_time_secs * 1000;

    Some(Candle {
        exchange: ExchangeId::Kucoin,
        symbol,
        open: Decimal::from_str(&row[1]).ok()?,
        high: Decimal::from_str(&row[3]).ok()?,
        low: Decimal::from_str(&row[4]).ok()?,
        close: Decimal::from_str(&row[2]).ok()?,
        volume: Decimal::from_str(&row[5]).ok()?,
        open_time_ms,
        close_time_ms: 0,
        is_closed: true, // KuCoin REST returns only closed candles
    })
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

/// Bullet token response (POST /api/v1/bullet-public)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinBulletResponse {
    pub token: String,
    pub instance_servers: Vec<KucoinInstanceServer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinInstanceServer {
    pub endpoint: String,
    #[serde(default)]
    pub ping_interval: Option<u64>,
    #[serde(default)]
    pub ping_timeout: Option<u64>,
}

/// Generic WS message envelope
#[derive(Debug, Deserialize)]
pub struct KucoinWsMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// WS trade match data
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinWsTradeData {
    pub symbol: String,
    pub side: String,
    pub price: String,
    pub size: String,
    #[serde(default)]
    pub trade_id: Option<String>,
    #[serde(default)]
    pub time: Option<String>, // nanoseconds as string
}

impl KucoinWsTradeData {
    pub fn into_trade(self) -> Trade {
        let symbol = kucoin_pair_to_unified(&self.symbol);
        let ts_ns: u64 = self
            .time
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Trade {
            exchange: ExchangeId::Kucoin,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side: parse_side(&self.side),
            timestamp_ms: ts_ns / 1_000_000,
            trade_id: self.trade_id,
        }
    }
}

/// WS orderbook depth data
#[derive(Debug, Deserialize)]
pub struct KucoinWsDepthData {
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl KucoinWsDepthData {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::Kucoin,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.timestamp.unwrap_or(0),
            sequence: None,
        }
    }
}

/// WS candle data
#[derive(Debug, Deserialize)]
pub struct KucoinWsCandleData {
    pub symbol: String,
    pub candles: Vec<String>, // [time, open, close, high, low, volume, turnover]
    #[serde(default)]
    pub time: Option<u64>, // nanoseconds
}

impl KucoinWsCandleData {
    pub fn into_candle(self) -> Option<Candle> {
        let symbol = kucoin_pair_to_unified(&self.symbol);
        if self.candles.len() < 7 {
            return None;
        }
        let open_time_secs: u64 = self.candles[0].parse().ok()?;
        Some(Candle {
            exchange: ExchangeId::Kucoin,
            symbol,
            open: Decimal::from_str(&self.candles[1]).ok()?,
            high: Decimal::from_str(&self.candles[3]).ok()?,
            low: Decimal::from_str(&self.candles[4]).ok()?,
            close: Decimal::from_str(&self.candles[2]).ok()?,
            volume: Decimal::from_str(&self.candles[5]).ok()?,
            open_time_ms: open_time_secs * 1000,
            close_time_ms: 0,
            is_closed: false, // WS candles are live updates
        })
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub fn parse_levels(raw: &[Vec<String>]) -> Vec<Level> {
    raw.iter()
        .filter_map(|entry| {
            if entry.len() >= 2 {
                let price = Decimal::from_str(&entry[0]).ok()?;
                let qty = Decimal::from_str(&entry[1]).ok()?;
                Some(Level::new(price, qty))
            } else {
                None
            }
        })
        .collect()
}

fn parse_side(s: &str) -> Side {
    match s {
        "buy" | "Buy" | "BUY" => Side::Buy,
        _ => Side::Sell,
    }
}

fn parse_decimal_opt(s: Option<&str>) -> Decimal {
    s.and_then(|v| Decimal::from_str(v).ok())
        .unwrap_or_default()
}

/// Convert a unified Symbol to a KuCoin pair string (e.g. "BTC-USDT").
pub fn unified_to_kucoin(symbol: &Symbol) -> String {
    format!("{}-{}", symbol.base, symbol.quote)
}

/// Convert a KuCoin pair string (e.g. "BTC-USDT") to a unified Symbol.
pub fn kucoin_pair_to_unified(pair: &str) -> Symbol {
    match pair.split_once('-') {
        Some((base, quote)) => Symbol::new(base, quote),
        None => Symbol::new(pair, ""),
    }
}

/// Map a unified Interval to the KuCoin REST candle type string.
pub fn interval_to_kucoin(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1min",
        Interval::M1 => "1min",
        Interval::M3 => "3min",
        Interval::M5 => "5min",
        Interval::M15 => "15min",
        Interval::M30 => "30min",
        Interval::H1 => "1hour",
        Interval::H4 => "4hour",
        Interval::D1 => "1day",
        Interval::W1 => "1week",
    }
}

/// Map a unified Interval to the KuCoin WS candle topic suffix.
pub fn interval_to_kucoin_ws(interval: Interval) -> &'static str {
    interval_to_kucoin(interval) // KuCoin uses same strings for REST and WS
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_symbol_conversion() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_kucoin(&sym), "BTC-USDT");

        let sym2 = Symbol::new("ETH", "BTC");
        assert_eq!(unified_to_kucoin(&sym2), "ETH-BTC");

        let u1 = kucoin_pair_to_unified("BTC-USDT");
        assert_eq!(u1.base, "BTC");
        assert_eq!(u1.quote, "USDT");

        let u2 = kucoin_pair_to_unified("ETH-BTC");
        assert_eq!(u2.base, "ETH");
        assert_eq!(u2.quote, "BTC");
    }

    #[test]
    fn test_interval_mapping() {
        assert_eq!(interval_to_kucoin(Interval::M1), "1min");
        assert_eq!(interval_to_kucoin(Interval::M3), "3min");
        assert_eq!(interval_to_kucoin(Interval::M5), "5min");
        assert_eq!(interval_to_kucoin(Interval::M15), "15min");
        assert_eq!(interval_to_kucoin(Interval::M30), "30min");
        assert_eq!(interval_to_kucoin(Interval::H1), "1hour");
        assert_eq!(interval_to_kucoin(Interval::H4), "4hour");
        assert_eq!(interval_to_kucoin(Interval::D1), "1day");
        assert_eq!(interval_to_kucoin(Interval::W1), "1week");
    }

    #[test]
    fn test_decimal_precision() {
        assert_eq!(decimal_precision("0.00000001"), 8);
        assert_eq!(decimal_precision("0.01"), 2);
        assert_eq!(decimal_precision("1"), 0);
        assert_eq!(decimal_precision("0.1"), 1);
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw = KucoinOrderBookRaw {
            time: Some(1700000000000),
            sequence: Some("14610502970".into()),
            bids: vec![
                vec!["50000.00".into(), "1.0".into()],
                vec!["49999.00".into(), "0.5".into()],
            ],
            asks: vec![vec!["50001.00".into(), "2.0".into()]],
        };

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Kucoin);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert_eq!(ob.sequence, Some(14610502970));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_conversion() {
        let raw = KucoinTradeRaw {
            sequence: "1545896668571".into(),
            price: "50000.50".into(),
            size: "0.5".into(),
            side: "buy".into(),
            time: 1545904567062000000, // nanoseconds
        };

        let trade = raw.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::Kucoin);
        assert_eq!(trade.symbol.base, "BTC");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(50000.50));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("1545896668571".into()));
        assert_eq!(trade.timestamp_ms, 1545904567062);
    }

    #[test]
    fn test_trade_sell_side() {
        let raw = KucoinTradeRaw {
            sequence: "1".into(),
            price: "100.0".into(),
            size: "1.0".into(),
            side: "sell".into(),
            time: 1700000000000000000,
        };
        let trade = raw.into_trade(Symbol::new("ETH", "USDT"));
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ticker_stats_conversion() {
        let raw = KucoinTickerStatsRaw {
            symbol: Some("BTC-USDT".into()),
            last: Some("50000.00".into()),
            buy: Some("49999.00".into()),
            sell: Some("50001.00".into()),
            vol: Some("12345.678".into()),
            change_rate: Some("0.0221".into()),
            time: Some(1700000000000),
        };

        let ticker = raw.into_ticker(Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.exchange, ExchangeId::Kucoin);
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(2.21)));
        assert_eq!(ticker.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_all_ticker_conversion() {
        let raw = KucoinAllTickerRaw {
            symbol: "ETH-USDT".into(),
            last: Some("2000.50".into()),
            buy: Some("2000.00".into()),
            sell: Some("2001.00".into()),
            vol: Some("5000.0".into()),
            change_rate: Some("-0.015".into()),
        };

        let ticker = raw.into_ticker(1700000000000);
        assert_eq!(ticker.symbol.base, "ETH");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(2000.50));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(-1.5)));
    }

    #[test]
    fn test_parse_kline_row() {
        let row = vec![
            "1545904980".into(), // time
            "50000.00".into(),   // open
            "50100.00".into(),   // close
            "50200.00".into(),   // high
            "49900.00".into(),   // low
            "100.5".into(),      // volume
            "5025000.00".into(), // turnover
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::Kucoin);
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1545904980000);
        assert!(candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row_too_short() {
        let row = vec!["1700000000".into(), "50000.00".into()];
        assert!(parse_kline_row(&row, Symbol::new("BTC", "USDT")).is_none());
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw = KucoinWsTradeData {
            symbol: "BTC-USDT".into(),
            side: "buy".into(),
            price: "50000.00".into(),
            size: "0.01".into(),
            trade_id: Some("abc123".into()),
            time: Some("1545913818099033203".into()),
        };

        let trade = raw.into_trade();
        assert_eq!(trade.symbol.base, "BTC");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(50000.00));
        assert_eq!(trade.qty, dec!(0.01));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("abc123".into()));
        assert_eq!(trade.timestamp_ms, 1545913818099);
    }

    #[test]
    fn test_ws_depth_conversion() {
        let raw = KucoinWsDepthData {
            asks: vec![vec!["50001.00".into(), "2.0".into()]],
            bids: vec![vec!["50000.00".into(), "1.0".into()]],
            timestamp: Some(1700000000000),
        };

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Kucoin);
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_candle_conversion() {
        let raw = KucoinWsCandleData {
            symbol: "BTC-USDT".into(),
            candles: vec![
                "1589968800".into(),
                "9786.9".into(),
                "9740.8".into(),
                "9806.1".into(),
                "9732".into(),
                "27.45649579".into(),
                "267856.624701952".into(),
            ],
            time: Some(1589970010253893337),
        };

        let candle = raw.into_candle().unwrap();
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.symbol.quote, "USDT");
        assert_eq!(candle.open, dec!(9786.9));
        assert_eq!(candle.close, dec!(9740.8));
        assert_eq!(candle.high, dec!(9806.1));
        assert_eq!(candle.low, dec!(9732));
        assert_eq!(candle.volume, dec!(27.45649579));
        assert_eq!(candle.open_time_ms, 1589968800000);
        assert!(!candle.is_closed);
    }

    #[test]
    fn test_ws_candle_too_short() {
        let raw = KucoinWsCandleData {
            symbol: "BTC-USDT".into(),
            candles: vec!["1589968800".into()],
            time: None,
        };
        assert!(raw.into_candle().is_none());
    }

    #[test]
    fn test_parse_levels() {
        let raw = vec![
            vec!["100.50".into(), "1.5".into()],
            vec!["99.00".into(), "2.0".into()],
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
            vec!["bad".into(), "1.0".into()],
            vec!["50.00".into(), "3.0".into()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, dec!(50.00));
    }

    #[test]
    fn test_exchange_info_conversion() {
        let symbols = vec![
            KucoinSymbolRaw {
                symbol: "BTC-USDT".into(),
                name: Some("BTC-USDT".into()),
                base_currency: "BTC".into(),
                quote_currency: "USDT".into(),
                base_min_size: Some("0.00001".into()),
                quote_min_size: Some("0.1".into()),
                base_increment: Some("0.00000001".into()),
                quote_increment: Some("0.000001".into()),
                price_increment: Some("0.1".into()),
                enable_trading: Some(true),
            },
            KucoinSymbolRaw {
                symbol: "ETH-USDT".into(),
                name: Some("ETH-USDT".into()),
                base_currency: "ETH".into(),
                quote_currency: "USDT".into(),
                base_min_size: Some("0.001".into()),
                quote_min_size: Some("0.1".into()),
                base_increment: Some("0.0001".into()),
                quote_increment: Some("0.01".into()),
                price_increment: Some("0.01".into()),
                enable_trading: Some(false),
            },
        ];

        let info = symbols_to_exchange_info(symbols);
        assert_eq!(info.exchange, ExchangeId::Kucoin);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTC-USDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 8);
        assert_eq!(btc.quote_precision, 6);
        assert_eq!(btc.min_qty, Some(dec!(0.00001)));
        assert_eq!(btc.min_notional, Some(dec!(0.1)));
        assert_eq!(btc.tick_size, Some(dec!(0.1)));

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::Halted);
        assert_eq!(eth.base_precision, 4);
    }
}
