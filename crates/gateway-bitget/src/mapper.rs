use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Response wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetResponse<T> {
    pub code: String,
    pub msg: String,
    pub data: T,
}

// ---------------------------------------------------------------------------
// Symbols (GET /api/v2/spot/public/symbols)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetSymbolRaw {
    pub symbol: String,
    #[serde(rename = "baseCoin")]
    pub base_coin: String,
    #[serde(rename = "quoteCoin")]
    pub quote_coin: String,
    #[serde(rename = "pricePrecision")]
    pub price_precision: String,
    #[serde(rename = "quantityPrecision")]
    pub quantity_precision: String,
    pub status: String,
    #[serde(rename = "minTradeUSDT")]
    pub min_trade_usdt: String,
}

pub fn symbols_to_exchange_info(symbols: Vec<BitgetSymbolRaw>) -> ExchangeInfo {
    let list = symbols
        .into_iter()
        .map(|raw| {
            let status = bitget_status_to_unified(&raw.status);
            let base_precision: u8 = raw.quantity_precision.parse().unwrap_or(0);
            let quote_precision: u8 = raw.price_precision.parse().unwrap_or(0);
            let min_notional = Decimal::from_str(&raw.min_trade_usdt).ok();

            SymbolInfo {
                symbol: Symbol::new(&raw.base_coin, &raw.quote_coin),
                raw_symbol: raw.symbol,
                status,
                base_precision,
                quote_precision,
                min_qty: None,
                min_notional,
                tick_size: None,
            }
        })
        .collect();

    ExchangeInfo {
        exchange: ExchangeId::Bitget,
        symbols: list,
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /api/v2/spot/market/orderbook)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetOrderBookData {
    pub asks: Vec<[String; 2]>,
    pub bids: Vec<[String; 2]>,
    pub ts: String,
}

impl BitgetOrderBookData {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::Bitget,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Trades (GET /api/v2/spot/market/fills)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetTradeRaw {
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    pub side: String,
    pub price: String,
    pub size: String,
    pub ts: String,
    pub symbol: String,
}

impl BitgetTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::Bitget,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Tickers (GET /api/v2/spot/market/tickers)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPr")]
    pub last_pr: String,
    #[serde(rename = "bidPr")]
    pub bid_pr: String,
    #[serde(rename = "askPr")]
    pub ask_pr: String,
    #[serde(rename = "baseVolume")]
    pub base_volume: String,
    #[serde(rename = "change24h")]
    pub change_24h: String,
    pub ts: String,
}

impl BitgetTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::Bitget,
            symbol,
            last_price: Decimal::from_str(&self.last_pr).unwrap_or_default(),
            bid: Decimal::from_str(&self.bid_pr).ok(),
            ask: Decimal::from_str(&self.ask_pr).ok(),
            volume_24h: Decimal::from_str(&self.base_volume).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.change_24h).ok(),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Klines (GET /api/v2/spot/market/candles)
// ---------------------------------------------------------------------------

/// Parse a single Bitget kline row into a Candle.
///
/// Row format: [ts, open, high, low, close, baseVol, quoteVol, usdtVol]
pub fn parse_kline_row(row: &[String], symbol: Symbol) -> Option<Candle> {
    if row.len() < 6 {
        return None;
    }
    let open_time_ms = row[0].parse::<u64>().ok()?;
    let open = Decimal::from_str(&row[1]).ok()?;
    let high = Decimal::from_str(&row[2]).ok()?;
    let low = Decimal::from_str(&row[3]).ok()?;
    let close = Decimal::from_str(&row[4]).ok()?;
    let volume = Decimal::from_str(&row[5]).ok()?;

    Some(Candle {
        exchange: ExchangeId::Bitget,
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
pub struct BitgetWsTradeRaw {
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    pub side: String,
    pub price: String,
    pub size: String,
    pub ts: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
}

impl BitgetWsTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = bitget_symbol_to_unified(&self.inst_id);
        let side = match self.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::Bitget,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BitgetWsOrderBook {
    pub asks: Vec<[String; 2]>,
    pub bids: Vec<[String; 2]>,
    pub ts: String,
    #[serde(default)]
    pub seq: Option<String>,
}

impl BitgetWsOrderBook {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        let seq = self.seq.as_deref().and_then(|s| s.parse::<u64>().ok());
        OrderBook {
            exchange: ExchangeId::Bitget,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            sequence: seq,
        }
    }
}

/// Parse a WS candle array: [ts, open, high, low, close, baseVol, quoteVol, usdtVol]
pub fn parse_ws_kline(row: &[String], symbol: Symbol) -> Option<Candle> {
    parse_kline_row(row, symbol)
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

/// Convert a unified Symbol to a Bitget raw symbol string (e.g. "BTCUSDT").
pub fn unified_to_bitget(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Known quote suffixes in priority order (longest first to avoid partial matches).
const KNOWN_QUOTES: &[&str] = &["USDT", "USDC", "BTC", "ETH", "DAI", "EUR"];

/// Convert a Bitget raw symbol string (e.g. "BTCUSDT") to a unified Symbol.
pub fn bitget_symbol_to_unified(raw: &str) -> Symbol {
    let upper = raw.to_uppercase();
    for quote in KNOWN_QUOTES {
        if upper.ends_with(quote) {
            let base = &upper[..upper.len() - quote.len()];
            if !base.is_empty() {
                return Symbol::new(base, *quote);
            }
        }
    }
    Symbol::new(&upper, "")
}

/// Map a unified Interval to the Bitget REST interval string.
pub fn interval_to_bitget_rest(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1min", // S1 unsupported, fallback to 1min
        Interval::M1 => "1min",
        Interval::M3 => "3min",
        Interval::M5 => "5min",
        Interval::M15 => "15min",
        Interval::M30 => "30min",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1day",
        Interval::W1 => "1week",
    }
}

/// Map a unified Interval to the Bitget WS channel string.
pub fn interval_to_bitget_ws(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "candle1m", // S1 unsupported, fallback to 1m
        Interval::M1 => "candle1m",
        Interval::M3 => "candle3m",
        Interval::M5 => "candle5m",
        Interval::M15 => "candle15m",
        Interval::M30 => "candle30m",
        Interval::H1 => "candle1H",
        Interval::H4 => "candle4H",
        Interval::D1 => "candle1D",
        Interval::W1 => "candle1W",
    }
}

/// Map a Bitget status string to a unified SymbolStatus.
pub fn bitget_status_to_unified(status: &str) -> SymbolStatus {
    match status {
        "online" => SymbolStatus::Trading,
        "halt" => SymbolStatus::Halted,
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
    fn test_interval_to_bitget_rest() {
        assert_eq!(interval_to_bitget_rest(Interval::S1), "1min");
        assert_eq!(interval_to_bitget_rest(Interval::M1), "1min");
        assert_eq!(interval_to_bitget_rest(Interval::M3), "3min");
        assert_eq!(interval_to_bitget_rest(Interval::M5), "5min");
        assert_eq!(interval_to_bitget_rest(Interval::M15), "15min");
        assert_eq!(interval_to_bitget_rest(Interval::M30), "30min");
        assert_eq!(interval_to_bitget_rest(Interval::H1), "1h");
        assert_eq!(interval_to_bitget_rest(Interval::H4), "4h");
        assert_eq!(interval_to_bitget_rest(Interval::D1), "1day");
        assert_eq!(interval_to_bitget_rest(Interval::W1), "1week");
    }

    #[test]
    fn test_interval_to_bitget_ws() {
        assert_eq!(interval_to_bitget_ws(Interval::S1), "candle1m");
        assert_eq!(interval_to_bitget_ws(Interval::M1), "candle1m");
        assert_eq!(interval_to_bitget_ws(Interval::M5), "candle5m");
        assert_eq!(interval_to_bitget_ws(Interval::H1), "candle1H");
        assert_eq!(interval_to_bitget_ws(Interval::H4), "candle4H");
        assert_eq!(interval_to_bitget_ws(Interval::D1), "candle1D");
        assert_eq!(interval_to_bitget_ws(Interval::W1), "candle1W");
    }

    #[test]
    fn test_symbol_conversion() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_bitget(&sym), "BTCUSDT");

        let sym2 = Symbol::new("ETH", "BTC");
        assert_eq!(unified_to_bitget(&sym2), "ETHBTC");

        let u1 = bitget_symbol_to_unified("BTCUSDT");
        assert_eq!(u1.base, "BTC");
        assert_eq!(u1.quote, "USDT");

        let u2 = bitget_symbol_to_unified("ETHBTC");
        assert_eq!(u2.base, "ETH");
        assert_eq!(u2.quote, "BTC");

        let u3 = bitget_symbol_to_unified("SOLUSDC");
        assert_eq!(u3.base, "SOL");
        assert_eq!(u3.quote, "USDC");

        let u4 = bitget_symbol_to_unified("BTCDAI");
        assert_eq!(u4.base, "BTC");
        assert_eq!(u4.quote, "DAI");

        let u5 = bitget_symbol_to_unified("ETHEUR");
        assert_eq!(u5.base, "ETH");
        assert_eq!(u5.quote, "EUR");
    }

    #[test]
    fn test_bitget_status() {
        assert_eq!(bitget_status_to_unified("online"), SymbolStatus::Trading);
        assert_eq!(bitget_status_to_unified("halt"), SymbolStatus::Halted);
        assert_eq!(bitget_status_to_unified("other"), SymbolStatus::Unknown);
        assert_eq!(bitget_status_to_unified(""), SymbolStatus::Unknown);
    }

    #[test]
    fn test_response_wrapper_deserialization() {
        let raw: BitgetResponse<Vec<serde_json::Value>> = serde_json::from_str(
            r#"{
                "code": "00000",
                "msg": "success",
                "data": [{"key": "value"}]
            }"#,
        )
        .unwrap();

        assert_eq!(raw.code, "00000");
        assert_eq!(raw.msg, "success");
        assert_eq!(raw.data.len(), 1);
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw: BitgetOrderBookData = serde_json::from_str(
            r#"{
                "asks": [["50001.00", "2.0"]],
                "bids": [["50000.00", "1.0"], ["49999.00", "0.5"]],
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Bitget);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert!(ob.sequence.is_none());
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_conversion() {
        let raw: BitgetTradeRaw = serde_json::from_str(
            r#"{
                "tradeId": "abc123",
                "symbol": "ETHUSDT",
                "price": "2000.50",
                "size": "0.5",
                "side": "buy",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::Bitget);
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
        let raw = BitgetTradeRaw {
            trade_id: "def456".to_string(),
            symbol: "BTCUSDT".to_string(),
            price: "50000.00".to_string(),
            size: "0.01".to_string(),
            side: "sell".to_string(),
            ts: "1700000000001".to_string(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ticker_conversion() {
        let raw: BitgetTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPr": "50000.00",
                "bidPr": "49999.00",
                "askPr": "50001.00",
                "baseVolume": "12345.678",
                "change24h": "0.025",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::Bitget);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(0.025)));
        assert_eq!(ticker.timestamp_ms, 1700000000000);
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
            "5025000.00".to_string(),
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::Bitget);
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
    fn test_ws_trade_conversion() {
        let raw: BitgetWsTradeRaw = serde_json::from_str(
            r#"{
                "tradeId": "trade-999",
                "instId": "ETHUSDT",
                "price": "2000.00",
                "size": "0.5",
                "side": "sell",
                "ts": "1700000000000"
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
    fn test_ws_orderbook_conversion() {
        let raw: BitgetWsOrderBook = serde_json::from_str(
            r#"{
                "asks": [["50001.00", "2.0"]],
                "bids": [["50000.00", "1.0"]],
                "ts": "1700000000000",
                "seq": "100"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::Bitget);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(100));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_orderbook_no_seq() {
        let raw: BitgetWsOrderBook = serde_json::from_str(
            r#"{
                "asks": [],
                "bids": [],
                "ts": "0"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert!(ob.sequence.is_none());
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
    fn test_exchange_info_conversion() {
        let symbols = vec![
            BitgetSymbolRaw {
                symbol: "BTCUSDT".to_string(),
                base_coin: "BTC".to_string(),
                quote_coin: "USDT".to_string(),
                price_precision: "2".to_string(),
                quantity_precision: "6".to_string(),
                status: "online".to_string(),
                min_trade_usdt: "1".to_string(),
            },
            BitgetSymbolRaw {
                symbol: "ETHUSDT".to_string(),
                base_coin: "ETH".to_string(),
                quote_coin: "USDT".to_string(),
                price_precision: "2".to_string(),
                quantity_precision: "4".to_string(),
                status: "halt".to_string(),
                min_trade_usdt: "5".to_string(),
            },
        ];

        let info = symbols_to_exchange_info(symbols);
        assert_eq!(info.exchange, ExchangeId::Bitget);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTCUSDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 6);
        assert_eq!(btc.quote_precision, 2);
        assert_eq!(btc.min_notional, Some(dec!(1)));
        assert!(btc.min_qty.is_none());
        assert!(btc.tick_size.is_none());

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::Halted);
    }
}
