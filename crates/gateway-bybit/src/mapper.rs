use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Response wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitResponse<T> {
    #[serde(rename = "retCode")]
    pub ret_code: i32,
    #[serde(rename = "retMsg")]
    pub ret_msg: String,
    pub result: T,
}

// ---------------------------------------------------------------------------
// Instruments Info (GET /v5/market/instruments-info)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitInstrumentsResult {
    pub category: String,
    pub list: Vec<BybitInstrumentRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitInstrumentRaw {
    pub symbol: String,
    #[serde(rename = "baseCoin")]
    pub base_coin: String,
    #[serde(rename = "quoteCoin")]
    pub quote_coin: String,
    pub status: String,
    #[serde(rename = "lotSizeFilter")]
    pub lot_size_filter: BybitLotSizeFilter,
    #[serde(rename = "priceFilter")]
    pub price_filter: BybitPriceFilter,
}

#[derive(Debug, Deserialize)]
pub struct BybitLotSizeFilter {
    #[serde(rename = "basePrecision")]
    pub base_precision: String,
    #[serde(rename = "quotePrecision")]
    pub quote_precision: String,
    #[serde(rename = "minOrderQty")]
    pub min_order_qty: String,
}

#[derive(Debug, Deserialize)]
pub struct BybitPriceFilter {
    #[serde(rename = "tickSize")]
    pub tick_size: String,
}

impl BybitInstrumentsResult {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .list
            .into_iter()
            .map(|raw| {
                let status = bybit_status_to_unified(&raw.status);
                let base_precision = decimal_precision(&raw.lot_size_filter.base_precision);
                let quote_precision = decimal_precision(&raw.lot_size_filter.quote_precision);
                let min_qty = Decimal::from_str(&raw.lot_size_filter.min_order_qty).ok();
                let tick_size = Decimal::from_str(&raw.price_filter.tick_size).ok();

                SymbolInfo {
                    symbol: Symbol::new(&raw.base_coin, &raw.quote_coin),
                    raw_symbol: raw.symbol,
                    status,
                    base_precision,
                    quote_precision,
                    min_qty,
                    min_notional: None,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::Bybit,
            symbols,
        }
    }
}

/// Compute the number of decimal places from a precision string like "0.001".
fn decimal_precision(s: &str) -> u8 {
    match s.find('.') {
        Some(pos) => {
            let decimals = &s[pos + 1..];
            decimals.len() as u8
        }
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// REST OrderBook (GET /v5/market/orderbook)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitOrderBookResult {
    pub s: String,
    pub b: Vec<[String; 2]>,
    pub a: Vec<[String; 2]>,
    pub u: u64,
    pub ts: u64,
}

impl BybitOrderBookResult {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = bybit_symbol_to_unified(&self.s);
        OrderBook {
            exchange: ExchangeId::Bybit,
            symbol,
            bids: parse_levels(&self.b),
            asks: parse_levels(&self.a),
            timestamp_ms: self.ts,
            sequence: Some(self.u),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Trades (GET /v5/market/recent-trade)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitTradesResult {
    pub category: String,
    pub list: Vec<BybitTradeRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitTradeRaw {
    #[serde(rename = "execId")]
    pub exec_id: String,
    pub symbol: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub time: String,
}

impl BybitTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::Bybit,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.time.parse::<u64>().unwrap_or(0),
            trade_id: Some(self.exec_id),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Tickers (GET /v5/market/tickers)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitTickersResult {
    pub category: String,
    pub list: Vec<BybitTickerRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "bid1Price")]
    pub bid1_price: String,
    #[serde(rename = "ask1Price")]
    pub ask1_price: String,
    #[serde(rename = "volume24h")]
    pub volume_24h: String,
    #[serde(rename = "price24hPcnt")]
    pub price_24h_pcnt: String,
    #[serde(rename = "highPrice24h")]
    pub high_price_24h: String,
    #[serde(rename = "lowPrice24h")]
    pub low_price_24h: String,
    #[serde(rename = "turnover24h")]
    pub turnover_24h: String,
}

impl BybitTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::Bybit,
            symbol,
            last_price: Decimal::from_str(&self.last_price).unwrap_or_default(),
            bid: Decimal::from_str(&self.bid1_price).ok(),
            ask: Decimal::from_str(&self.ask1_price).ok(),
            volume_24h: Decimal::from_str(&self.volume_24h).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.price_24h_pcnt).ok(),
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// REST Klines (GET /v5/market/kline)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitKlinesResult {
    pub category: String,
    pub symbol: String,
    pub list: Vec<Vec<String>>,
}

/// Parse a single Bybit kline row into a Candle.
///
/// Row format: [startTime, open, high, low, close, volume, turnover]
pub fn parse_kline_row(row: &[String], symbol: Symbol) -> Option<Candle> {
    if row.len() < 7 {
        return None;
    }
    let open_time_ms = row[0].parse::<u64>().ok()?;
    let open = Decimal::from_str(&row[1]).ok()?;
    let high = Decimal::from_str(&row[2]).ok()?;
    let low = Decimal::from_str(&row[3]).ok()?;
    let close = Decimal::from_str(&row[4]).ok()?;
    let volume = Decimal::from_str(&row[5]).ok()?;

    Some(Candle {
        exchange: ExchangeId::Bybit,
        symbol,
        open,
        high,
        low,
        close,
        volume,
        open_time_ms,
        close_time_ms: 0,
        is_closed: true,
    })
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitWsMessage<T> {
    pub topic: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub ts: u64,
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct BybitWsOrderBook {
    pub s: String,
    pub b: Vec<[String; 2]>,
    pub a: Vec<[String; 2]>,
    pub u: u64,
    pub seq: u64,
}

impl BybitWsOrderBook {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = bybit_symbol_to_unified(&self.s);
        OrderBook {
            exchange: ExchangeId::Bybit,
            symbol,
            bids: parse_levels(&self.b),
            asks: parse_levels(&self.a),
            timestamp_ms: 0,
            sequence: Some(self.u),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BybitWsTrade {
    #[serde(rename = "T")]
    pub trade_time: u64,
    pub s: String,
    #[serde(rename = "S")]
    pub side: String,
    pub v: String,
    pub p: String,
    pub i: String,
}

impl BybitWsTrade {
    pub fn into_trade(self) -> Trade {
        let symbol = bybit_symbol_to_unified(&self.s);
        let side = match self.side.as_str() {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::Bybit,
            symbol,
            price: Decimal::from_str(&self.p).unwrap_or_default(),
            qty: Decimal::from_str(&self.v).unwrap_or_default(),
            side,
            timestamp_ms: self.trade_time,
            trade_id: Some(self.i),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BybitWsKlineData {
    pub start: u64,
    pub end: u64,
    pub interval: String,
    pub open: String,
    pub close: String,
    pub high: String,
    pub low: String,
    pub volume: String,
    pub confirm: bool,
}

impl BybitWsKlineData {
    pub fn into_candle(self, symbol: Symbol) -> Candle {
        Candle {
            exchange: ExchangeId::Bybit,
            symbol,
            open: Decimal::from_str(&self.open).unwrap_or_default(),
            high: Decimal::from_str(&self.high).unwrap_or_default(),
            low: Decimal::from_str(&self.low).unwrap_or_default(),
            close: Decimal::from_str(&self.close).unwrap_or_default(),
            volume: Decimal::from_str(&self.volume).unwrap_or_default(),
            open_time_ms: self.start,
            close_time_ms: self.end,
            is_closed: self.confirm,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BybitWsTicker {
    pub symbol: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "bid1Price")]
    pub bid1_price: String,
    #[serde(rename = "ask1Price")]
    pub ask1_price: String,
    #[serde(rename = "volume24h")]
    pub volume_24h: String,
    #[serde(rename = "price24hPcnt")]
    pub price_24h_pcnt: String,
}

#[derive(Debug, Deserialize)]
pub struct BybitWsResponse {
    pub op: String,
    pub success: Option<bool>,
    #[serde(rename = "conn_id")]
    pub conn_id: Option<String>,
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

/// Convert a unified Symbol to a Bybit raw symbol string (e.g. "BTCUSDT").
pub fn unified_to_bybit(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Known quote suffixes in priority order (longest first to avoid partial matches).
const KNOWN_QUOTES: &[&str] = &["USDT", "USDC", "BTC", "ETH", "DAI", "EUR"];

/// Convert a Bybit raw symbol string (e.g. "BTCUSDT") to a unified Symbol.
pub fn bybit_symbol_to_unified(raw: &str) -> Symbol {
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

/// Map a unified Interval to the Bybit interval string.
pub fn interval_to_bybit(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1",
        Interval::M1 => "1",
        Interval::M3 => "3",
        Interval::M5 => "5",
        Interval::M15 => "15",
        Interval::M30 => "30",
        Interval::H1 => "60",
        Interval::H4 => "240",
        Interval::D1 => "D",
        Interval::W1 => "W",
    }
}

/// Map a Bybit status string to a unified SymbolStatus.
pub fn bybit_status_to_unified(status: &str) -> SymbolStatus {
    match status {
        "Trading" => SymbolStatus::Trading,
        "PreLaunch" => SymbolStatus::PreTrading,
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
    fn test_interval_to_bybit() {
        assert_eq!(interval_to_bybit(Interval::S1), "1");
        assert_eq!(interval_to_bybit(Interval::M1), "1");
        assert_eq!(interval_to_bybit(Interval::M3), "3");
        assert_eq!(interval_to_bybit(Interval::M5), "5");
        assert_eq!(interval_to_bybit(Interval::M15), "15");
        assert_eq!(interval_to_bybit(Interval::M30), "30");
        assert_eq!(interval_to_bybit(Interval::H1), "60");
        assert_eq!(interval_to_bybit(Interval::H4), "240");
        assert_eq!(interval_to_bybit(Interval::D1), "D");
        assert_eq!(interval_to_bybit(Interval::W1), "W");
    }

    #[test]
    fn test_symbol_conversion() {
        // unified -> bybit
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_bybit(&sym), "BTCUSDT");

        let sym2 = Symbol::new("ETH", "BTC");
        assert_eq!(unified_to_bybit(&sym2), "ETHBTC");

        // bybit -> unified
        let u1 = bybit_symbol_to_unified("BTCUSDT");
        assert_eq!(u1.base, "BTC");
        assert_eq!(u1.quote, "USDT");

        let u2 = bybit_symbol_to_unified("ETHBTC");
        assert_eq!(u2.base, "ETH");
        assert_eq!(u2.quote, "BTC");

        let u3 = bybit_symbol_to_unified("SOLUSDC");
        assert_eq!(u3.base, "SOL");
        assert_eq!(u3.quote, "USDC");

        let u4 = bybit_symbol_to_unified("BTCDAI");
        assert_eq!(u4.base, "BTC");
        assert_eq!(u4.quote, "DAI");

        let u5 = bybit_symbol_to_unified("ETHEUR");
        assert_eq!(u5.base, "ETH");
        assert_eq!(u5.quote, "EUR");
    }

    #[test]
    fn test_bybit_status() {
        assert_eq!(bybit_status_to_unified("Trading"), SymbolStatus::Trading);
        assert_eq!(bybit_status_to_unified("PreLaunch"), SymbolStatus::PreTrading);
        assert_eq!(bybit_status_to_unified("Settling"), SymbolStatus::Unknown);
        assert_eq!(bybit_status_to_unified(""), SymbolStatus::Unknown);
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw: BybitOrderBookResult = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "b": [["50000.00", "1.0"], ["49999.00", "0.5"]],
                "a": [["50001.00", "2.0"]],
                "u": 42,
                "ts": 1700000000000
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::Bybit);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.bids[1].price, dec!(49999.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert_eq!(ob.sequence, Some(42));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_parse_kline_row() {
        let row = vec![
            "1700000000000".to_string(),
            "50000.00".to_string(),
            "50200.00".to_string(),
            "49900.00".to_string(),
            "50100.00".to_string(),
            "100.5".to_string(),
            "5025000.00".to_string(),
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::Bybit);
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
    fn test_parse_kline_row_too_short() {
        let row = vec!["1700000000000".to_string(), "50000.00".to_string()];
        assert!(parse_kline_row(&row, Symbol::new("BTC", "USDT")).is_none());
    }

    #[test]
    fn test_ticker_conversion() {
        let raw: BybitTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPrice": "50000.00",
                "bid1Price": "49999.00",
                "ask1Price": "50001.00",
                "volume24h": "12345.678",
                "price24hPcnt": "0.025",
                "highPrice24h": "51000.00",
                "lowPrice24h": "49000.00",
                "turnover24h": "617283900.00"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::Bybit);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(0.025)));
    }

    #[test]
    fn test_trade_conversion() {
        let raw: BybitTradeRaw = serde_json::from_str(
            r#"{
                "execId": "abc123",
                "symbol": "ETHUSDT",
                "price": "2000.50",
                "size": "0.5",
                "side": "Buy",
                "time": "1700000000000"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::Bybit);
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.50));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("abc123".to_string()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_sell_side() {
        let raw = BybitTradeRaw {
            exec_id: "def456".to_string(),
            symbol: "BTCUSDT".to_string(),
            price: "50000.00".to_string(),
            size: "0.01".to_string(),
            side: "Sell".to_string(),
            time: "1700000000001".to_string(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ws_orderbook_conversion() {
        let raw: BybitWsOrderBook = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "b": [["50000.00", "1.0"]],
                "a": [["50001.00", "2.0"]],
                "u": 100,
                "seq": 200
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::Bybit);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(100));
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw: BybitWsTrade = serde_json::from_str(
            r#"{
                "T": 1700000000000,
                "s": "ETHUSDT",
                "S": "Sell",
                "v": "0.5",
                "p": "2000.00",
                "i": "trade-999"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.00));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.trade_id, Some("trade-999".to_string()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_kline_conversion() {
        let raw: BybitWsKlineData = serde_json::from_str(
            r#"{
                "start": 1700000000000,
                "end": 1700000060000,
                "interval": "1",
                "open": "50000.00",
                "close": "50100.00",
                "high": "50200.00",
                "low": "49900.00",
                "volume": "100.5",
                "confirm": true
            }"#,
        )
        .unwrap();

        let candle = raw.into_candle(Symbol::new("BTC", "USDT"));
        assert_eq!(candle.exchange, ExchangeId::Bybit);
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000);
        assert!(candle.is_closed);
    }

    #[test]
    fn test_ws_kline_not_confirmed() {
        let raw = BybitWsKlineData {
            start: 1700000000000,
            end: 1700000060000,
            interval: "1".to_string(),
            open: "50000.00".to_string(),
            close: "50050.00".to_string(),
            high: "50100.00".to_string(),
            low: "49950.00".to_string(),
            volume: "10.0".to_string(),
            confirm: false,
        };
        let candle = raw.into_candle(Symbol::new("BTC", "USDT"));
        assert!(!candle.is_closed);
    }

    #[test]
    fn test_response_wrapper_deserialization() {
        let raw: BybitResponse<BybitOrderBookResult> = serde_json::from_str(
            r#"{
                "retCode": 0,
                "retMsg": "OK",
                "result": {
                    "s": "BTCUSDT",
                    "b": [["50000.00", "1.0"]],
                    "a": [["50001.00", "2.0"]],
                    "u": 42,
                    "ts": 1700000000000
                }
            }"#,
        )
        .unwrap();

        assert_eq!(raw.ret_code, 0);
        assert_eq!(raw.ret_msg, "OK");
        assert_eq!(raw.result.s, "BTCUSDT");
    }

    #[test]
    fn test_exchange_info_conversion() {
        let raw: BybitResponse<BybitInstrumentsResult> = serde_json::from_str(
            r#"{
                "retCode": 0,
                "retMsg": "OK",
                "result": {
                    "category": "spot",
                    "list": [{
                        "symbol": "BTCUSDT",
                        "baseCoin": "BTC",
                        "quoteCoin": "USDT",
                        "status": "Trading",
                        "lotSizeFilter": {
                            "basePrecision": "0.000001",
                            "quotePrecision": "0.00000001",
                            "minOrderQty": "0.000048"
                        },
                        "priceFilter": {
                            "tickSize": "0.01"
                        }
                    }, {
                        "symbol": "ETHUSDT",
                        "baseCoin": "ETH",
                        "quoteCoin": "USDT",
                        "status": "PreLaunch",
                        "lotSizeFilter": {
                            "basePrecision": "0.00001",
                            "quotePrecision": "0.0000001",
                            "minOrderQty": "0.00067"
                        },
                        "priceFilter": {
                            "tickSize": "0.01"
                        }
                    }]
                }
            }"#,
        )
        .unwrap();

        let info = raw.result.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::Bybit);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTCUSDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 6);
        assert_eq!(btc.quote_precision, 8);
        assert_eq!(btc.min_qty, Some(dec!(0.000048)));
        assert_eq!(btc.tick_size, Some(dec!(0.01)));
        assert!(btc.min_notional.is_none());

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::PreTrading);
    }

    #[test]
    fn test_ws_response_deserialization() {
        let raw: BybitWsResponse = serde_json::from_str(
            r#"{
                "op": "subscribe",
                "success": true,
                "conn_id": "abc-123"
            }"#,
        )
        .unwrap();

        assert_eq!(raw.op, "subscribe");
        assert_eq!(raw.success, Some(true));
        assert_eq!(raw.conn_id, Some("abc-123".to_string()));
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
    fn test_decimal_precision() {
        assert_eq!(decimal_precision("0.000001"), 6);
        assert_eq!(decimal_precision("0.00000001"), 8);
        assert_eq!(decimal_precision("0.01"), 2);
        assert_eq!(decimal_precision("1"), 0);
    }
}
