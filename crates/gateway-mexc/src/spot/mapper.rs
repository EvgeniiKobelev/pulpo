use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbols (GET /api/v3/exchangeInfo)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcExchangeInfoResponse {
    pub symbols: Vec<MexcSymbolRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcSymbolRaw {
    pub symbol: String,
    #[serde(default)]
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    #[serde(default)]
    pub base_asset_precision: Option<u8>,
    #[serde(default)]
    pub quote_precision: Option<u8>,
    #[serde(default)]
    pub quote_asset_precision: Option<u8>,
    #[serde(default)]
    pub is_spot_trading_allowed: Option<bool>,
}

pub fn symbols_to_exchange_info(resp: MexcExchangeInfoResponse) -> ExchangeInfo {
    let list = resp
        .symbols
        .into_iter()
        .map(|raw| {
            let status = mexc_status_to_unified(&raw.status);
            let base_precision = raw.base_asset_precision.unwrap_or(8);
            let quote_precision = raw
                .quote_precision
                .or(raw.quote_asset_precision)
                .unwrap_or(8);

            SymbolInfo {
                symbol: Symbol::new(&raw.base_asset, &raw.quote_asset),
                raw_symbol: raw.symbol,
                status,
                base_precision,
                quote_precision,
                min_qty: None,
                min_notional: None,
                tick_size: None,
            }
        })
        .collect();

    ExchangeInfo {
        exchange: ExchangeId::Mexc,
        symbols: list,
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /api/v3/depth)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcOrderBookRaw {
    pub last_update_id: Option<u64>,
    pub bids: Vec<Vec<String>>,
    pub asks: Vec<Vec<String>>,
}

impl MexcOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::Mexc,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: 0,
            sequence: self.last_update_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Trades (GET /api/v3/trades)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcTradeRaw {
    pub id: Option<serde_json::Value>,
    pub price: String,
    pub qty: String,
    #[serde(default)]
    pub quote_qty: Option<String>,
    pub time: u64,
    #[serde(default)]
    pub is_buyer_maker: bool,
}

impl MexcTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let side = if self.is_buyer_maker {
            Side::Sell
        } else {
            Side::Buy
        };
        Trade {
            exchange: ExchangeId::Mexc,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side,
            timestamp_ms: self.time,
            trade_id: self.id.map(|v| v.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Klines (GET /api/v3/klines)
// ---------------------------------------------------------------------------

/// Parse a MEXC kline row (Binance-compatible format).
///
/// Row format: [open_time, open, high, low, close, volume, close_time, quote_volume]
pub fn parse_kline_row(row: &[serde_json::Value], symbol: Symbol) -> Option<Candle> {
    if row.len() < 7 {
        return None;
    }
    let open_time_ms = row[0].as_u64()?;
    let open = decimal_from_value(&row[1])?;
    let high = decimal_from_value(&row[2])?;
    let low = decimal_from_value(&row[3])?;
    let close = decimal_from_value(&row[4])?;
    let volume = decimal_from_value(&row[5])?;
    let close_time_ms = row[6].as_u64().unwrap_or(0);

    Some(Candle {
        exchange: ExchangeId::Mexc,
        symbol,
        open,
        high,
        low,
        close,
        volume,
        open_time_ms,
        close_time_ms,
        is_closed: true,
    })
}

// ---------------------------------------------------------------------------
// Tickers (GET /api/v3/ticker/24hr)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcTickerRaw {
    pub symbol: String,
    #[serde(default)]
    pub last_price: Option<String>,
    #[serde(default)]
    pub bid_price: Option<String>,
    #[serde(default)]
    pub ask_price: Option<String>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub price_change_percent: Option<String>,
    #[serde(default)]
    pub open_price: Option<String>,
    #[serde(default)]
    pub high_price: Option<String>,
    #[serde(default)]
    pub low_price: Option<String>,
}

impl MexcTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = mexc_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::Mexc,
            symbol,
            last_price: self
                .last_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            bid: self
                .bid_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok()),
            ask: self
                .ask_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok()),
            volume_24h: self
                .volume
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            price_change_pct_24h: self
                .price_change_percent
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok()),
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

/// Outer wrapper for all MEXC WS v3 data messages.
///
/// Format: `{"d": {...}, "c": "channel", "t": timestamp, "s": "SYMBOL"}`
#[derive(Debug, Deserialize)]
pub struct MexcWsMessage {
    pub d: serde_json::Value,
    pub c: String,
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub s: Option<String>,
}

/// MEXC WS subscription confirmation / pong.
///
/// Format: `{"id": 0, "code": 0, "msg": "PONG"}` or `{"id": 0, "code": 0, "msg": "spot@..."}`
#[derive(Debug, Deserialize)]
pub struct MexcWsControl {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub code: Option<i64>,
    #[serde(default)]
    pub msg: Option<String>,
}

/// MEXC WS deals (trades) payload inside "d".
#[derive(Debug, Deserialize)]
pub struct MexcWsDeals {
    pub deals: Vec<MexcWsDeal>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcWsDeal {
    pub p: Option<String>,
    pub v: Option<String>,
    #[serde(alias = "S")]
    pub trade_type: Option<i32>,
    pub t: Option<u64>,
    // Fallback field names from proto JSON encoding
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub time: Option<u64>,
}

impl MexcWsDeal {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let price_str = self.p.or(self.price).unwrap_or_default();
        let qty_str = self.v.or(self.quantity).unwrap_or_default();
        let ts = self.t.or(self.time).unwrap_or(0);
        let trade_type = self.trade_type.unwrap_or(0);
        // tradeType: 1=buy, 2=sell
        let side = if trade_type == 1 { Side::Buy } else { Side::Sell };
        Trade {
            exchange: ExchangeId::Mexc,
            symbol,
            price: Decimal::from_str(&price_str).unwrap_or_default(),
            qty: Decimal::from_str(&qty_str).unwrap_or_default(),
            side,
            timestamp_ms: ts,
            trade_id: None,
        }
    }
}

/// MEXC WS depth payload inside "d".
#[derive(Debug, Deserialize)]
pub struct MexcWsDepth {
    pub asks: Vec<MexcWsDepthLevel>,
    pub bids: Vec<MexcWsDepthLevel>,
    #[serde(default, alias = "r")]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MexcWsDepthLevel {
    pub p: Option<String>,
    pub v: Option<String>,
    // Fallback field names from proto JSON encoding
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub quantity: Option<String>,
}

impl MexcWsDepth {
    pub fn into_orderbook(self, symbol: Symbol, timestamp_ms: u64) -> OrderBook {
        let seq = self.version.and_then(|v| v.parse::<u64>().ok());
        OrderBook {
            exchange: ExchangeId::Mexc,
            symbol,
            bids: self
                .bids
                .into_iter()
                .filter_map(|l| l.into_level())
                .collect(),
            asks: self
                .asks
                .into_iter()
                .filter_map(|l| l.into_level())
                .collect(),
            timestamp_ms,
            sequence: seq,
        }
    }
}

impl MexcWsDepthLevel {
    pub fn into_level(self) -> Option<Level> {
        let p = self.p.or(self.price)?;
        let q = self.v.or(self.quantity)?;
        let price = Decimal::from_str(&p).ok()?;
        let qty = Decimal::from_str(&q).ok()?;
        Some(Level::new(price, qty))
    }
}

/// MEXC WS kline payload inside "d".
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcWsKline {
    pub k: Option<MexcWsKlineData>,
    // Fallback: data directly in "d" (proto JSON format)
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub window_start: Option<i64>,
    #[serde(default)]
    pub opening_price: Option<String>,
    #[serde(default)]
    pub closing_price: Option<String>,
    #[serde(default)]
    pub highest_price: Option<String>,
    #[serde(default)]
    pub lowest_price: Option<String>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub window_end: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MexcWsKlineData {
    #[serde(default)]
    pub t: Option<i64>,
    #[serde(default)]
    pub o: Option<String>,
    #[serde(default)]
    pub c: Option<String>,
    #[serde(default)]
    pub h: Option<String>,
    #[serde(default)]
    pub l: Option<String>,
    #[serde(default)]
    pub v: Option<String>,
    #[serde(default, alias = "T")]
    pub end_time: Option<i64>,
    #[serde(default)]
    pub i: Option<String>,
}

impl MexcWsKline {
    pub fn into_candle(self, symbol: Symbol) -> Option<Candle> {
        // Try nested "k" format first
        if let Some(k) = self.k {
            let open = Decimal::from_str(k.o.as_deref()?).ok()?;
            let high = Decimal::from_str(k.h.as_deref()?).ok()?;
            let low = Decimal::from_str(k.l.as_deref()?).ok()?;
            let close = Decimal::from_str(k.c.as_deref()?).ok()?;
            let volume = k
                .v
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default();
            return Some(Candle {
                exchange: ExchangeId::Mexc,
                symbol,
                open,
                high,
                low,
                close,
                volume,
                open_time_ms: k.t.unwrap_or(0) as u64,
                close_time_ms: k.end_time.unwrap_or(0) as u64,
                is_closed: false,
            });
        }
        // Fallback: proto JSON format (fields directly in "d")
        let open = Decimal::from_str(self.opening_price.as_deref()?).ok()?;
        let high = Decimal::from_str(self.highest_price.as_deref()?).ok()?;
        let low = Decimal::from_str(self.lowest_price.as_deref()?).ok()?;
        let close = Decimal::from_str(self.closing_price.as_deref()?).ok()?;
        let volume = self
            .volume
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        Some(Candle {
            exchange: ExchangeId::Mexc,
            symbol,
            open,
            high,
            low,
            close,
            volume,
            open_time_ms: self.window_start.unwrap_or(0) as u64,
            close_time_ms: self.window_end.unwrap_or(0) as u64,
            is_closed: false,
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

fn decimal_from_value(v: &serde_json::Value) -> Option<Decimal> {
    match v {
        serde_json::Value::String(s) => Decimal::from_str(s).ok(),
        serde_json::Value::Number(n) => {
            let s = n.to_string();
            Decimal::from_str(&s).ok()
        }
        _ => None,
    }
}

/// Convert a unified Symbol to a MEXC symbol string (e.g. "BTCUSDT").
pub fn unified_to_mexc(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Convert a MEXC symbol string (e.g. "BTCUSDT") to a unified Symbol.
///
/// MEXC uses concatenated format without separator — we use the same
/// approach as Binance, matching known quote assets.
pub fn mexc_to_unified(raw: &str) -> Symbol {
    let upper = raw.to_uppercase();
    for quote in KNOWN_QUOTES {
        if upper.ends_with(quote) {
            let base = &upper[..upper.len() - quote.len()];
            if !base.is_empty() {
                return Symbol::new(base, *quote);
            }
        }
    }
    Symbol::new(raw, "")
}

const KNOWN_QUOTES: &[&str] = &[
    "USDT", "USDC", "BUSD", "TUSD", "FDUSD", "BTC", "ETH", "BNB", "DAI", "EUR", "GBP", "TRY",
    "BRL", "ARS",
];

/// Map a unified Interval to the MEXC REST kline interval string.
pub fn interval_to_mexc_rest(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1m",
        Interval::M1 => "1m",
        Interval::M3 => "5m",
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "60m",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "1W",
    }
}

/// Map a unified Interval to the MEXC WS kline interval string.
pub fn interval_to_mexc_ws(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "Min1",
        Interval::M1 => "Min1",
        Interval::M3 => "Min5",
        Interval::M5 => "Min5",
        Interval::M15 => "Min15",
        Interval::M30 => "Min30",
        Interval::H1 => "Min60",
        Interval::H4 => "Hour4",
        Interval::D1 => "Day1",
        Interval::W1 => "Week",
    }
}

/// Map a MEXC symbol status string to a unified SymbolStatus.
pub fn mexc_status_to_unified(status: &str) -> SymbolStatus {
    match status {
        "1" | "ENABLED" | "TRADING" => SymbolStatus::Trading,
        "2" | "HALT" | "BREAK" => SymbolStatus::Halted,
        "PRE_TRADING" => SymbolStatus::PreTrading,
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
        assert_eq!(unified_to_mexc(&sym), "BTCUSDT");

        let sym2 = Symbol::new("ETH", "BTC");
        assert_eq!(unified_to_mexc(&sym2), "ETHBTC");
    }

    #[test]
    fn test_mexc_to_unified() {
        let u1 = mexc_to_unified("BTCUSDT");
        assert_eq!(u1.base, "BTC");
        assert_eq!(u1.quote, "USDT");

        let u2 = mexc_to_unified("ETHBTC");
        assert_eq!(u2.base, "ETH");
        assert_eq!(u2.quote, "BTC");

        let u3 = mexc_to_unified("SOLUSDC");
        assert_eq!(u3.base, "SOL");
        assert_eq!(u3.quote, "USDC");
    }

    #[test]
    fn test_interval_to_mexc_rest() {
        assert_eq!(interval_to_mexc_rest(Interval::M1), "1m");
        assert_eq!(interval_to_mexc_rest(Interval::M5), "5m");
        assert_eq!(interval_to_mexc_rest(Interval::M15), "15m");
        assert_eq!(interval_to_mexc_rest(Interval::M30), "30m");
        assert_eq!(interval_to_mexc_rest(Interval::H1), "60m");
        assert_eq!(interval_to_mexc_rest(Interval::H4), "4h");
        assert_eq!(interval_to_mexc_rest(Interval::D1), "1d");
        assert_eq!(interval_to_mexc_rest(Interval::W1), "1W");
    }

    #[test]
    fn test_interval_to_mexc_ws() {
        assert_eq!(interval_to_mexc_ws(Interval::M1), "Min1");
        assert_eq!(interval_to_mexc_ws(Interval::M5), "Min5");
        assert_eq!(interval_to_mexc_ws(Interval::M15), "Min15");
        assert_eq!(interval_to_mexc_ws(Interval::M30), "Min30");
        assert_eq!(interval_to_mexc_ws(Interval::H1), "Min60");
        assert_eq!(interval_to_mexc_ws(Interval::H4), "Hour4");
        assert_eq!(interval_to_mexc_ws(Interval::D1), "Day1");
        assert_eq!(interval_to_mexc_ws(Interval::W1), "Week");
    }

    #[test]
    fn test_mexc_status() {
        assert_eq!(mexc_status_to_unified("1"), SymbolStatus::Trading);
        assert_eq!(mexc_status_to_unified("ENABLED"), SymbolStatus::Trading);
        assert_eq!(mexc_status_to_unified("TRADING"), SymbolStatus::Trading);
        assert_eq!(mexc_status_to_unified("2"), SymbolStatus::Halted);
        assert_eq!(mexc_status_to_unified("HALT"), SymbolStatus::Halted);
        assert_eq!(mexc_status_to_unified("BREAK"), SymbolStatus::Halted);
        assert_eq!(
            mexc_status_to_unified("PRE_TRADING"),
            SymbolStatus::PreTrading
        );
        assert_eq!(mexc_status_to_unified("other"), SymbolStatus::Unknown);
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw = MexcOrderBookRaw {
            last_update_id: Some(1027024),
            bids: vec![
                vec!["50000.00".into(), "1.0".into()],
                vec!["49999.00".into(), "0.5".into()],
            ],
            asks: vec![vec!["50001.00".into(), "2.0".into()]],
        };

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Mexc);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert_eq!(ob.sequence, Some(1027024));
    }

    #[test]
    fn test_trade_conversion() {
        let raw = MexcTradeRaw {
            id: Some(serde_json::Value::Number(serde_json::Number::from(123456))),
            price: "24990.00".into(),
            qty: "0.10".into(),
            quote_qty: None,
            time: 1678886400000,
            is_buyer_maker: true,
        };

        let trade = raw.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::Mexc);
        assert_eq!(trade.symbol.base, "BTC");
        assert_eq!(trade.price, dec!(24990.00));
        assert_eq!(trade.qty, dec!(0.10));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.timestamp_ms, 1678886400000);
    }

    #[test]
    fn test_trade_buyer_side() {
        let raw = MexcTradeRaw {
            id: None,
            price: "100.0".into(),
            qty: "1.0".into(),
            quote_qty: None,
            time: 1700000000000,
            is_buyer_maker: false,
        };
        let trade = raw.into_trade(Symbol::new("ETH", "USDT"));
        assert_eq!(trade.side, Side::Buy);
    }

    #[test]
    fn test_ticker_conversion() {
        let raw = MexcTickerRaw {
            symbol: "BTCUSDT".into(),
            last_price: Some("50000.00".into()),
            bid_price: Some("49999.00".into()),
            ask_price: Some("50001.00".into()),
            volume: Some("12345.678".into()),
            price_change_percent: Some("0.00400048".into()),
            open_price: Some("46079.37".into()),
            high_price: Some("47550.01".into()),
            low_price: Some("45555.5".into()),
        };

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::Mexc);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
    }

    #[test]
    fn test_parse_kline_row() {
        let row = vec![
            serde_json::json!(1640804880000u64),
            serde_json::json!("47482.36"),
            serde_json::json!("47482.36"),
            serde_json::json!("47416.57"),
            serde_json::json!("47436.1"),
            serde_json::json!("3.550717"),
            serde_json::json!(1640804940000u64),
            serde_json::json!("168387.3"),
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::Mexc);
        assert_eq!(candle.open, dec!(47482.36));
        assert_eq!(candle.high, dec!(47482.36));
        assert_eq!(candle.low, dec!(47416.57));
        assert_eq!(candle.close, dec!(47436.1));
        assert_eq!(candle.volume, dec!(3.550717));
        assert_eq!(candle.open_time_ms, 1640804880000);
        assert_eq!(candle.close_time_ms, 1640804940000);
    }

    #[test]
    fn test_parse_kline_row_too_short() {
        let row = vec![serde_json::json!(1640804880000u64), serde_json::json!("100")];
        assert!(parse_kline_row(&row, Symbol::new("BTC", "USDT")).is_none());
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
    fn test_ws_deal_conversion() {
        let deal = MexcWsDeal {
            p: Some("36474.74".into()),
            v: Some("0.001".into()),
            trade_type: Some(1),
            t: Some(1699502456050),
            price: None,
            quantity: None,
            time: None,
        };

        let trade = deal.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.price, dec!(36474.74));
        assert_eq!(trade.qty, dec!(0.001));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.timestamp_ms, 1699502456050);
    }

    #[test]
    fn test_ws_deal_proto_format() {
        let deal = MexcWsDeal {
            p: None,
            v: None,
            trade_type: Some(2),
            t: None,
            price: Some("50000.00".into()),
            quantity: Some("0.5".into()),
            time: Some(1700000000000),
        };

        let trade = deal.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.price, dec!(50000.00));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_depth_conversion() {
        let depth = MexcWsDepth {
            asks: vec![MexcWsDepthLevel {
                p: Some("50001.00".into()),
                v: Some("2.0".into()),
                price: None,
                quantity: None,
            }],
            bids: vec![MexcWsDepthLevel {
                p: Some("50000.00".into()),
                v: Some("1.0".into()),
                price: None,
                quantity: None,
            }],
            version: Some("12345".into()),
        };

        let ob = depth.into_orderbook(Symbol::new("BTC", "USDT"), 1700000000000);
        assert_eq!(ob.exchange, ExchangeId::Mexc);
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(12345));
    }

    #[test]
    fn test_exchange_info_conversion() {
        let resp = MexcExchangeInfoResponse {
            symbols: vec![MexcSymbolRaw {
                symbol: "BTCUSDT".into(),
                status: "1".into(),
                base_asset: "BTC".into(),
                quote_asset: "USDT".into(),
                base_asset_precision: Some(8),
                quote_precision: Some(2),
                quote_asset_precision: None,
                is_spot_trading_allowed: Some(true),
            }],
        };

        let info = symbols_to_exchange_info(resp);
        assert_eq!(info.exchange, ExchangeId::Mexc);
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.symbols[0].raw_symbol, "BTCUSDT");
        assert_eq!(info.symbols[0].status, SymbolStatus::Trading);
        assert_eq!(info.symbols[0].base_precision, 8);
        assert_eq!(info.symbols[0].quote_precision, 2);
    }
}
