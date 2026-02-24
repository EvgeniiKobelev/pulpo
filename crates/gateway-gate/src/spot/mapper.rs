use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbols (GET /spot/currency_pairs)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateSymbolRaw {
    pub id: String,
    pub base: String,
    pub quote: String,
    #[serde(default)]
    pub fee: String,
    #[serde(default)]
    pub min_base_amount: Option<String>,
    #[serde(default)]
    pub min_quote_amount: Option<String>,
    #[serde(default)]
    pub amount_precision: Option<u8>,
    #[serde(default)]
    pub precision: Option<u8>,
    #[serde(default)]
    pub trade_status: String,
}

pub fn symbols_to_exchange_info(symbols: Vec<GateSymbolRaw>) -> ExchangeInfo {
    let list = symbols
        .into_iter()
        .map(|raw| {
            let status = gate_status_to_unified(&raw.trade_status);
            let base_precision = raw.amount_precision.unwrap_or(0);
            let quote_precision = raw.precision.unwrap_or(0);
            let min_qty = raw
                .min_base_amount
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok());
            let min_notional = raw
                .min_quote_amount
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok());

            SymbolInfo {
                symbol: Symbol::new(&raw.base, &raw.quote),
                raw_symbol: raw.id,
                status,
                base_precision,
                quote_precision,
                min_qty,
                min_notional,
                tick_size: None,
            }
        })
        .collect();

    ExchangeInfo {
        exchange: ExchangeId::Gate,
        symbols: list,
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /spot/order_book)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateOrderBookRaw {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub current: Option<u64>,
    #[serde(default)]
    pub update: Option<u64>,
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
}

impl GateOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        let ts = self.current.or(self.update).unwrap_or(0);
        OrderBook {
            exchange: ExchangeId::Gate,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: ts,
            sequence: self.id,
        }
    }
}

// ---------------------------------------------------------------------------
// Trades (GET /spot/trades)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateTradeRaw {
    pub id: String,
    #[serde(default)]
    pub create_time_ms: Option<String>,
    #[serde(default)]
    pub create_time: Option<String>,
    pub currency_pair: String,
    pub side: String,
    pub amount: String,
    pub price: String,
}

impl GateTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = gate_pair_to_unified(&self.currency_pair);
        let ts = self
            .create_time_ms
            .as_deref()
            .and_then(|s| {
                // Gate returns "1771859436665.477000" — take integer part
                s.split('.').next().and_then(|i| i.parse::<u64>().ok())
            })
            .or_else(|| {
                self.create_time
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok().map(|t| t * 1000))
            })
            .unwrap_or(0);
        Trade {
            exchange: ExchangeId::Gate,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.amount).unwrap_or_default(),
            side: parse_side(&self.side),
            timestamp_ms: ts,
            trade_id: Some(self.id),
        }
    }
}

// ---------------------------------------------------------------------------
// Tickers (GET /spot/tickers)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateTickerRaw {
    pub currency_pair: String,
    pub last: String,
    #[serde(default)]
    pub lowest_ask: String,
    #[serde(default)]
    pub highest_bid: String,
    #[serde(default)]
    pub change_percentage: String,
    #[serde(default)]
    pub base_volume: String,
    #[serde(default)]
    pub high_24h: String,
    #[serde(default)]
    pub low_24h: String,
}

impl GateTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = gate_pair_to_unified(&self.currency_pair);
        Ticker {
            exchange: ExchangeId::Gate,
            symbol,
            last_price: Decimal::from_str(&self.last).unwrap_or_default(),
            bid: Decimal::from_str(&self.highest_bid).ok(),
            ask: Decimal::from_str(&self.lowest_ask).ok(),
            volume_24h: Decimal::from_str(&self.base_volume).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.change_percentage).ok(),
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Klines (GET /spot/candlesticks)
// ---------------------------------------------------------------------------

/// Parse a Gate.io kline row.
///
/// Row format: [timestamp, quote_vol, close, high, low, open, base_vol, is_window_closed]
/// NOTE: this is NOT standard OHLCV order!
pub fn parse_kline_row(row: &[String], symbol: Symbol) -> Option<Candle> {
    if row.len() < 8 {
        return None;
    }
    let open_time_secs: u64 = row[0].parse().ok()?;
    let open_time_ms = open_time_secs * 1000;
    let close = Decimal::from_str(&row[2]).ok()?;
    let high = Decimal::from_str(&row[3]).ok()?;
    let low = Decimal::from_str(&row[4]).ok()?;
    let open = Decimal::from_str(&row[5]).ok()?;
    let volume = Decimal::from_str(&row[6]).ok()?;
    let is_closed = row[7] == "true";

    Some(Candle {
        exchange: ExchangeId::Gate,
        symbol,
        open,
        high,
        low,
        close,
        volume,
        open_time_ms,
        close_time_ms: 0,
        is_closed,
    })
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateWsTradeResult {
    pub id: Option<u64>,
    pub create_time_ms: Option<String>,
    pub create_time: Option<u64>,
    pub side: String,
    pub currency_pair: String,
    pub amount: String,
    pub price: String,
}

impl GateWsTradeResult {
    pub fn into_trade(self) -> Trade {
        let symbol = gate_pair_to_unified(&self.currency_pair);
        let ts = self
            .create_time_ms
            .as_deref()
            .and_then(|s| s.split('.').next().and_then(|i| i.parse::<u64>().ok()))
            .or_else(|| self.create_time.map(|t| t * 1000))
            .unwrap_or(0);
        Trade {
            exchange: ExchangeId::Gate,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.amount).unwrap_or_default(),
            side: parse_side(&self.side),
            timestamp_ms: ts,
            trade_id: self.id.map(|i| i.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GateWsOrderBookResult {
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default, rename = "lastUpdateId")]
    pub last_update_id: Option<u64>,
    #[serde(default)]
    pub s: Option<String>,
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
}

impl GateWsOrderBookResult {
    pub fn into_orderbook(self, fallback_symbol: Symbol) -> OrderBook {
        let symbol = self
            .s
            .as_deref()
            .map(gate_pair_to_unified)
            .unwrap_or(fallback_symbol);
        OrderBook {
            exchange: ExchangeId::Gate,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.t.unwrap_or(0),
            sequence: self.last_update_id,
        }
    }
}

/// Gate.io WS candle result.
///
/// Fields: t=timestamp(s), v=quote_vol, c=close, h=high, l=low, o=open, n=name, a=base_vol, w=window_closed
#[derive(Debug, Deserialize)]
pub struct GateWsCandleResult {
    pub t: String,
    #[serde(default)]
    pub v: String,
    pub c: String,
    pub h: String,
    pub l: String,
    pub o: String,
    pub n: String,
    #[serde(default)]
    pub a: String,
    #[serde(default)]
    pub w: bool,
}

impl GateWsCandleResult {
    pub fn into_candle(self) -> Option<Candle> {
        // n format: "1m_BTC_USDT"
        let pair = self.n.splitn(2, '_').nth(1)?;
        let symbol = gate_pair_to_unified(pair);
        let open_time_secs: u64 = self.t.parse().ok()?;
        Some(Candle {
            exchange: ExchangeId::Gate,
            symbol,
            open: Decimal::from_str(&self.o).ok()?,
            high: Decimal::from_str(&self.h).ok()?,
            low: Decimal::from_str(&self.l).ok()?,
            close: Decimal::from_str(&self.c).ok()?,
            volume: Decimal::from_str(&self.a).unwrap_or_default(),
            open_time_ms: open_time_secs * 1000,
            close_time_ms: 0,
            is_closed: self.w,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GateWsTickerResult {
    pub currency_pair: String,
    pub last: String,
    #[serde(default)]
    pub lowest_ask: String,
    #[serde(default)]
    pub highest_bid: String,
    #[serde(default)]
    pub change_percentage: String,
    #[serde(default)]
    pub base_volume: String,
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
        "buy" | "Buy" => Side::Buy,
        _ => Side::Sell,
    }
}

/// Convert a unified Symbol to a Gate.io pair string (e.g. "BTC_USDT").
pub fn unified_to_gate(symbol: &Symbol) -> String {
    format!("{}_{}", symbol.base, symbol.quote)
}

/// Convert a Gate.io pair string (e.g. "BTC_USDT") to a unified Symbol.
pub fn gate_pair_to_unified(pair: &str) -> Symbol {
    match pair.split_once('_') {
        Some((base, quote)) => Symbol::new(base, quote),
        None => Symbol::new(pair, ""),
    }
}

/// Map a unified Interval to the Gate.io REST candlestick interval string.
pub fn interval_to_gate_rest(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1s",
        Interval::M1 => "1m",
        Interval::M3 => "5m", // Gate has no 3m, use 5m as fallback
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "7d",
    }
}

/// Map a unified Interval to the Gate.io WS candlestick interval string.
pub fn interval_to_gate_ws(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1m", // Gate WS has no 1s, fallback to 1m
        Interval::M1 => "1m",
        Interval::M3 => "5m", // Gate WS has no 3m, fallback to 5m
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "7d",
    }
}

/// Map a Gate.io trade status string to a unified SymbolStatus.
pub fn gate_status_to_unified(status: &str) -> SymbolStatus {
    match status {
        "tradable" => SymbolStatus::Trading,
        "untradable" => SymbolStatus::Halted,
        _ => SymbolStatus::Unknown,
    }
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
        assert_eq!(unified_to_gate(&sym), "BTC_USDT");

        let sym2 = Symbol::new("ETH", "BTC");
        assert_eq!(unified_to_gate(&sym2), "ETH_BTC");

        let u1 = gate_pair_to_unified("BTC_USDT");
        assert_eq!(u1.base, "BTC");
        assert_eq!(u1.quote, "USDT");

        let u2 = gate_pair_to_unified("ETH_BTC");
        assert_eq!(u2.base, "ETH");
        assert_eq!(u2.quote, "BTC");
    }

    #[test]
    fn test_interval_to_gate_rest() {
        assert_eq!(interval_to_gate_rest(Interval::S1), "1s");
        assert_eq!(interval_to_gate_rest(Interval::M1), "1m");
        assert_eq!(interval_to_gate_rest(Interval::M5), "5m");
        assert_eq!(interval_to_gate_rest(Interval::M15), "15m");
        assert_eq!(interval_to_gate_rest(Interval::M30), "30m");
        assert_eq!(interval_to_gate_rest(Interval::H1), "1h");
        assert_eq!(interval_to_gate_rest(Interval::H4), "4h");
        assert_eq!(interval_to_gate_rest(Interval::D1), "1d");
        assert_eq!(interval_to_gate_rest(Interval::W1), "7d");
    }

    #[test]
    fn test_interval_to_gate_ws() {
        assert_eq!(interval_to_gate_ws(Interval::S1), "1m");
        assert_eq!(interval_to_gate_ws(Interval::M1), "1m");
        assert_eq!(interval_to_gate_ws(Interval::M5), "5m");
        assert_eq!(interval_to_gate_ws(Interval::H1), "1h");
        assert_eq!(interval_to_gate_ws(Interval::H4), "4h");
        assert_eq!(interval_to_gate_ws(Interval::D1), "1d");
        assert_eq!(interval_to_gate_ws(Interval::W1), "7d");
    }

    #[test]
    fn test_gate_status() {
        assert_eq!(gate_status_to_unified("tradable"), SymbolStatus::Trading);
        assert_eq!(gate_status_to_unified("untradable"), SymbolStatus::Halted);
        assert_eq!(gate_status_to_unified("other"), SymbolStatus::Unknown);
        assert_eq!(gate_status_to_unified(""), SymbolStatus::Unknown);
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw = GateOrderBookRaw {
            id: Some(12345),
            current: Some(1700000000000),
            update: Some(1699999999999),
            asks: vec![vec!["50001.00".into(), "2.0".into()]],
            bids: vec![
                vec!["50000.00".into(), "1.0".into()],
                vec!["49999.00".into(), "0.5".into()],
            ],
        };

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Gate);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert_eq!(ob.sequence, Some(12345));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_conversion() {
        let raw = GateTradeRaw {
            id: "177409494".into(),
            create_time_ms: Some("1700000000123.456".into()),
            create_time: Some("1700000000".into()),
            currency_pair: "ETH_USDT".into(),
            side: "buy".into(),
            amount: "0.5".into(),
            price: "2000.50".into(),
        };

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::Gate);
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.50));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("177409494".into()));
        assert_eq!(trade.timestamp_ms, 1700000000123);
    }

    #[test]
    fn test_trade_sell_side() {
        let raw = GateTradeRaw {
            id: "1".into(),
            create_time_ms: None,
            create_time: Some("1700000000".into()),
            currency_pair: "BTC_USDT".into(),
            side: "sell".into(),
            amount: "0.01".into(),
            price: "50000.00".into(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ticker_conversion() {
        let raw = GateTickerRaw {
            currency_pair: "BTC_USDT".into(),
            last: "50000.00".into(),
            lowest_ask: "50001.00".into(),
            highest_bid: "49999.00".into(),
            change_percentage: "-2.21".into(),
            base_volume: "12345.678".into(),
            high_24h: "52000".into(),
            low_24h: "48000".into(),
        };

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::Gate);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(-2.21)));
    }

    #[test]
    fn test_parse_kline_row() {
        let row = vec![
            "1700000000".into(),
            "5025000.00".into(), // quote_vol
            "50100.00".into(),   // close
            "50200.00".into(),   // high
            "49900.00".into(),   // low
            "50000.00".into(),   // open
            "100.5".into(),      // base_vol
            "true".into(),       // is_closed
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::Gate);
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert!(candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row_not_closed() {
        let row = vec![
            "1700000000".into(),
            "100.0".into(),
            "50.0".into(),
            "51.0".into(),
            "49.0".into(),
            "50.5".into(),
            "2.0".into(),
            "false".into(),
        ];
        let candle = parse_kline_row(&row, Symbol::new("ETH", "USDT")).unwrap();
        assert!(!candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row_too_short() {
        let row = vec!["1700000000".into(), "50000.00".into()];
        assert!(parse_kline_row(&row, Symbol::new("BTC", "USDT")).is_none());
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw = GateWsTradeResult {
            id: Some(309143071),
            create_time_ms: Some("1606292218213.4578".into()),
            create_time: Some(1606292218),
            side: "sell".into(),
            currency_pair: "GT_USDT".into(),
            amount: "16.47".into(),
            price: "0.4705".into(),
        };

        let trade = raw.into_trade();
        assert_eq!(trade.symbol.base, "GT");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(0.4705));
        assert_eq!(trade.qty, dec!(16.47));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.trade_id, Some("309143071".into()));
        assert_eq!(trade.timestamp_ms, 1606292218213);
    }

    #[test]
    fn test_ws_orderbook_conversion() {
        let raw = GateWsOrderBookResult {
            t: Some(1700000000000),
            last_update_id: Some(48791820),
            s: Some("BTC_USDT".into()),
            asks: vec![vec!["50001.00".into(), "2.0".into()]],
            bids: vec![vec!["50000.00".into(), "1.0".into()]],
        };

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Gate);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(48791820));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_candle_conversion() {
        let raw = GateWsCandleResult {
            t: "1606292580".into(),
            v: "2362.32035".into(),
            c: "19128.1".into(),
            h: "19130.0".into(),
            l: "19125.0".into(),
            o: "19126.5".into(),
            n: "1m_BTC_USDT".into(),
            a: "3.8283".into(),
            w: true,
        };

        let candle = raw.into_candle().unwrap();
        assert_eq!(candle.exchange, ExchangeId::Gate);
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.symbol.quote, "USDT");
        assert_eq!(candle.open, dec!(19126.5));
        assert_eq!(candle.high, dec!(19130.0));
        assert_eq!(candle.low, dec!(19125.0));
        assert_eq!(candle.close, dec!(19128.1));
        assert_eq!(candle.volume, dec!(3.8283));
        assert_eq!(candle.open_time_ms, 1606292580000);
        assert!(candle.is_closed);
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
            GateSymbolRaw {
                id: "BTC_USDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
                fee: "0.2".into(),
                min_base_amount: Some("0.000001".into()),
                min_quote_amount: Some("3".into()),
                amount_precision: Some(6),
                precision: Some(1),
                trade_status: "tradable".into(),
            },
            GateSymbolRaw {
                id: "ETH_USDT".into(),
                base: "ETH".into(),
                quote: "USDT".into(),
                fee: "0.2".into(),
                min_base_amount: Some("0.001".into()),
                min_quote_amount: Some("3".into()),
                amount_precision: Some(4),
                precision: Some(2),
                trade_status: "untradable".into(),
            },
        ];

        let info = symbols_to_exchange_info(symbols);
        assert_eq!(info.exchange, ExchangeId::Gate);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTC_USDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 6);
        assert_eq!(btc.quote_precision, 1);
        assert_eq!(btc.min_qty, Some(dec!(0.000001)));
        assert_eq!(btc.min_notional, Some(dec!(3)));

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::Halted);
    }
}
