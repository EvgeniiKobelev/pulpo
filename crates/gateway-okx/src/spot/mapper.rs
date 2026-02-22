use gateway_core::{
    Candle, ExchangeId, Interval, Level, OrderBook, Side, Symbol, SymbolInfo, SymbolStatus, Ticker,
    Trade,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Response wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OkxResponse<T> {
    pub code: String,
    pub msg: String,
    pub data: Vec<T>,
}

// ---------------------------------------------------------------------------
// Symbol conversion
// ---------------------------------------------------------------------------

/// Converts a unified Symbol to OKX spot instrument ID: "BTC-USDT"
pub fn unified_to_okx(symbol: &Symbol) -> String {
    format!("{}-{}", symbol.base, symbol.quote)
}

/// Converts a unified Symbol to OKX perpetual swap instrument ID: "BTC-USDT-SWAP"
pub fn unified_to_okx_swap(symbol: &Symbol) -> String {
    format!("{}-{}-SWAP", symbol.base, symbol.quote)
}

/// Parses an OKX instrument ID to a unified Symbol.
/// Handles both "BTC-USDT" (spot) and "BTC-USDT-SWAP" (perpetual).
pub fn okx_inst_id_to_unified(inst_id: &str) -> Option<Symbol> {
    let parts: Vec<&str> = inst_id.split('-').collect();
    if parts.len() >= 2 {
        Some(Symbol::new(parts[0], parts[1]))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Interval mapping
// ---------------------------------------------------------------------------

pub fn interval_to_okx(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1s",
        Interval::M1 => "1m",
        Interval::M3 => "3m",
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1H",
        Interval::H4 => "4H",
        Interval::D1 => "1D",
        Interval::W1 => "1W",
    }
}

pub fn interval_to_okx_ws(interval: Interval) -> String {
    format!("candle{}", interval_to_okx(interval))
}

// ---------------------------------------------------------------------------
// Status mapping
// ---------------------------------------------------------------------------

fn okx_state_to_status(state: &str) -> SymbolStatus {
    match state {
        "live" => SymbolStatus::Trading,
        "suspend" => SymbolStatus::Halted,
        "preopen" => SymbolStatus::PreTrading,
        _ => SymbolStatus::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Precision helper
// ---------------------------------------------------------------------------

/// Calculates decimal precision from a tick/lot size string like "0.0001" → 4
fn precision_from_size(s: &str) -> u8 {
    if let Some(dot_pos) = s.find('.') {
        let after_dot = &s[dot_pos + 1..];
        let trimmed = after_dot.trim_end_matches('0');
        if trimmed.is_empty() {
            0
        } else {
            trimmed.len() as u8
        }
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// REST raw types — Instruments (exchange info)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxInstrumentRaw {
    pub inst_type: String,
    pub inst_id: String,
    #[serde(default)]
    pub base_ccy: String,
    #[serde(default)]
    pub quote_ccy: String,
    #[serde(default)]
    pub settle_ccy: String,
    pub tick_sz: String,
    pub lot_sz: String,
    pub min_sz: String,
    pub state: String,
    #[serde(default)]
    pub ct_val: String,
    #[serde(default)]
    pub ct_type: String,
}

impl OkxInstrumentRaw {
    pub fn into_symbol_info(self, _exchange: ExchangeId) -> Option<SymbolInfo> {
        let symbol = if !self.base_ccy.is_empty() && !self.quote_ccy.is_empty() {
            Symbol::new(&self.base_ccy, &self.quote_ccy)
        } else {
            okx_inst_id_to_unified(&self.inst_id)?
        };

        let base_precision = precision_from_size(&self.lot_sz);
        let quote_precision = precision_from_size(&self.tick_sz);
        let min_qty = Decimal::from_str(&self.min_sz).ok();
        let tick_size = Decimal::from_str(&self.tick_sz).ok();

        Some(SymbolInfo {
            symbol,
            raw_symbol: self.inst_id,
            status: okx_state_to_status(&self.state),
            base_precision,
            quote_precision,
            min_qty,
            min_notional: None,
            tick_size,
        })
    }
}

// ---------------------------------------------------------------------------
// REST raw types — OrderBook
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OkxOrderBookRaw {
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
    pub ts: String,
}

impl OkxOrderBookRaw {
    pub fn into_orderbook(self, exchange: ExchangeId, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.ts.parse().unwrap_or(0),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// REST raw types — Trade
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxTradeRaw {
    pub inst_id: String,
    pub trade_id: String,
    pub px: String,
    pub sz: String,
    pub side: String,
    pub ts: String,
}

impl OkxTradeRaw {
    pub fn into_trade(self, exchange: ExchangeId) -> Trade {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        Trade {
            exchange,
            symbol,
            price: Decimal::from_str(&self.px).unwrap_or_default(),
            qty: Decimal::from_str(&self.sz).unwrap_or_default(),
            side: parse_side(&self.side),
            timestamp_ms: self.ts.parse().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

// ---------------------------------------------------------------------------
// REST raw types — Ticker
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxTickerRaw {
    pub inst_id: String,
    pub last: String,
    pub ask_px: String,
    pub bid_px: String,
    #[serde(default)]
    pub open24h: String,
    pub vol24h: String,
    #[serde(default)]
    pub vol_ccy24h: String,
    pub ts: String,
    #[serde(default)]
    pub high24h: String,
    #[serde(default)]
    pub low24h: String,
}

impl OkxTickerRaw {
    pub fn into_ticker(self, exchange: ExchangeId) -> Ticker {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        let last = Decimal::from_str(&self.last).unwrap_or_default();
        let open = Decimal::from_str(&self.open24h).unwrap_or_default();
        let pct = if !open.is_zero() {
            Some((last - open) / open * Decimal::from(100))
        } else {
            None
        };
        Ticker {
            exchange,
            symbol,
            last_price: last,
            bid: Decimal::from_str(&self.bid_px).ok(),
            ask: Decimal::from_str(&self.ask_px).ok(),
            volume_24h: Decimal::from_str(&self.vol24h).unwrap_or_default(),
            price_change_pct_24h: pct,
            timestamp_ms: self.ts.parse().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// REST raw types — Candles (klines)
// ---------------------------------------------------------------------------

/// OKX returns candles as arrays: [ts, o, h, l, c, vol, volCcy, volCcyQuote, confirm]
pub fn parse_kline_row(
    row: &[String],
    exchange: ExchangeId,
    symbol: &Symbol,
    interval: Interval,
) -> Option<Candle> {
    if row.len() < 9 {
        return None;
    }
    let open_time: u64 = row[0].parse().ok()?;
    let close_time = open_time + interval.as_secs() * 1000;
    Some(Candle {
        exchange,
        symbol: symbol.clone(),
        open: Decimal::from_str(&row[1]).ok()?,
        high: Decimal::from_str(&row[2]).ok()?,
        low: Decimal::from_str(&row[3]).ok()?,
        close: Decimal::from_str(&row[4]).ok()?,
        volume: Decimal::from_str(&row[5]).ok()?,
        open_time_ms: open_time,
        close_time_ms: close_time,
        is_closed: row[8] == "1",
    })
}

// ---------------------------------------------------------------------------
// WebSocket raw types — OrderBook (books5)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OkxWsBookMsg {
    pub arg: OkxWsArg,
    pub data: Vec<OkxWsBookData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxWsArg {
    pub channel: String,
    #[serde(default)]
    pub inst_id: String,
    #[serde(default)]
    pub inst_type: String,
}

#[derive(Debug, Deserialize)]
pub struct OkxWsBookData {
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
    pub ts: String,
    #[serde(default)]
    pub checksum: Option<i64>,
    #[serde(default, rename = "seqId")]
    pub seq_id: Option<u64>,
}

// ---------------------------------------------------------------------------
// WebSocket raw types — Trade
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OkxWsTradeMsg {
    pub arg: OkxWsArg,
    pub data: Vec<OkxWsTradeData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxWsTradeData {
    pub inst_id: String,
    pub trade_id: String,
    pub px: String,
    pub sz: String,
    pub side: String,
    pub ts: String,
}

impl OkxWsTradeData {
    pub fn into_trade(self, exchange: ExchangeId) -> Trade {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        Trade {
            exchange,
            symbol,
            price: Decimal::from_str(&self.px).unwrap_or_default(),
            qty: Decimal::from_str(&self.sz).unwrap_or_default(),
            side: parse_side(&self.side),
            timestamp_ms: self.ts.parse().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket raw types — Candle
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OkxWsCandleMsg {
    pub arg: OkxWsArg,
    pub data: Vec<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_side(s: &str) -> Side {
    match s {
        "buy" => Side::Buy,
        _ => Side::Sell,
    }
}

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_unified_to_okx() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_okx(&sym), "BTC-USDT");
    }

    #[test]
    fn test_unified_to_okx_swap() {
        let sym = Symbol::new("ETH", "USDT");
        assert_eq!(unified_to_okx_swap(&sym), "ETH-USDT-SWAP");
    }

    #[test]
    fn test_okx_inst_id_to_unified_spot() {
        let sym = okx_inst_id_to_unified("BTC-USDT").unwrap();
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");
    }

    #[test]
    fn test_okx_inst_id_to_unified_swap() {
        let sym = okx_inst_id_to_unified("ETH-USDT-SWAP").unwrap();
        assert_eq!(sym.base, "ETH");
        assert_eq!(sym.quote, "USDT");
    }

    #[test]
    fn test_okx_inst_id_to_unified_invalid() {
        assert!(okx_inst_id_to_unified("INVALID").is_none());
    }

    #[test]
    fn test_precision_from_size() {
        assert_eq!(precision_from_size("0.0001"), 4);
        assert_eq!(precision_from_size("0.01"), 2);
        assert_eq!(precision_from_size("1"), 0);
        assert_eq!(precision_from_size("0.00000001"), 8);
        assert_eq!(precision_from_size("0.10"), 1);
    }

    #[test]
    fn test_interval_to_okx() {
        assert_eq!(interval_to_okx(Interval::M1), "1m");
        assert_eq!(interval_to_okx(Interval::H1), "1H");
        assert_eq!(interval_to_okx(Interval::D1), "1D");
        assert_eq!(interval_to_okx(Interval::W1), "1W");
    }

    #[test]
    fn test_interval_to_okx_ws() {
        assert_eq!(interval_to_okx_ws(Interval::M5), "candle5m");
        assert_eq!(interval_to_okx_ws(Interval::H4), "candle4H");
    }

    #[test]
    fn test_parse_levels() {
        let raw = vec![
            vec!["43000.1".into(), "0.5".into(), "0".into(), "3".into()],
            vec!["42999.9".into(), "1.2".into(), "0".into(), "5".into()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(43000.1));
        assert_eq!(levels[0].qty, dec!(0.5));
        assert_eq!(levels[1].price, dec!(42999.9));
        assert_eq!(levels[1].qty, dec!(1.2));
    }

    #[test]
    fn test_parse_side() {
        assert_eq!(parse_side("buy"), Side::Buy);
        assert_eq!(parse_side("sell"), Side::Sell);
        assert_eq!(parse_side("unknown"), Side::Sell);
    }

    #[test]
    fn test_okx_state_to_status() {
        assert_eq!(okx_state_to_status("live"), SymbolStatus::Trading);
        assert_eq!(okx_state_to_status("suspend"), SymbolStatus::Halted);
        assert_eq!(okx_state_to_status("preopen"), SymbolStatus::PreTrading);
        assert_eq!(okx_state_to_status("test"), SymbolStatus::Unknown);
    }

    #[test]
    fn test_trade_raw_into_trade() {
        let raw = OkxTradeRaw {
            inst_id: "BTC-USDT".into(),
            trade_id: "12345".into(),
            px: "43000.5".into(),
            sz: "0.1".into(),
            side: "buy".into(),
            ts: "1700000000000".into(),
        };
        let trade = raw.into_trade(ExchangeId::Okx);
        assert_eq!(trade.symbol.base, "BTC");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(43000.5));
        assert_eq!(trade.qty, dec!(0.1));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.timestamp_ms, 1700000000000);
        assert_eq!(trade.trade_id, Some("12345".into()));
    }

    #[test]
    fn test_ticker_raw_into_ticker() {
        let raw = OkxTickerRaw {
            inst_id: "ETH-USDT".into(),
            last: "2100.50".into(),
            ask_px: "2100.60".into(),
            bid_px: "2100.40".into(),
            open24h: "2000.00".into(),
            vol24h: "50000".into(),
            vol_ccy24h: "105000000".into(),
            ts: "1700000000000".into(),
            high24h: "2150.00".into(),
            low24h: "1980.00".into(),
        };
        let ticker = raw.into_ticker(ExchangeId::Okx);
        assert_eq!(ticker.symbol.base, "ETH");
        assert_eq!(ticker.last_price, dec!(2100.50));
        assert!(ticker.price_change_pct_24h.is_some());
    }

    #[test]
    fn test_parse_kline_row() {
        let row: Vec<String> = vec![
            "1700000000000".into(),
            "43000.0".into(),
            "43500.0".into(),
            "42800.0".into(),
            "43200.0".into(),
            "100.5".into(),
            "4321000".into(),
            "4321000".into(),
            "1".into(),
        ];
        let sym = Symbol::new("BTC", "USDT");
        let candle = parse_kline_row(&row, ExchangeId::Okx, &sym, Interval::H1).unwrap();
        assert_eq!(candle.open, dec!(43000.0));
        assert_eq!(candle.high, dec!(43500.0));
        assert_eq!(candle.low, dec!(42800.0));
        assert_eq!(candle.close, dec!(43200.0));
        assert_eq!(candle.volume, dec!(100.5));
        assert!(candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row_not_closed() {
        let row: Vec<String> = vec![
            "1700000000000".into(),
            "43000.0".into(),
            "43500.0".into(),
            "42800.0".into(),
            "43200.0".into(),
            "100.5".into(),
            "4321000".into(),
            "4321000".into(),
            "0".into(),
        ];
        let sym = Symbol::new("BTC", "USDT");
        let candle = parse_kline_row(&row, ExchangeId::Okx, &sym, Interval::M1).unwrap();
        assert!(!candle.is_closed);
    }

    #[test]
    fn test_instrument_raw_into_symbol_info() {
        let raw = OkxInstrumentRaw {
            inst_type: "SPOT".into(),
            inst_id: "BTC-USDT".into(),
            base_ccy: "BTC".into(),
            quote_ccy: "USDT".into(),
            settle_ccy: String::new(),
            tick_sz: "0.1".into(),
            lot_sz: "0.00001".into(),
            min_sz: "0.00001".into(),
            state: "live".into(),
            ct_val: String::new(),
            ct_type: String::new(),
        };
        let info = raw.into_symbol_info(ExchangeId::Okx).unwrap();
        assert_eq!(info.symbol.base, "BTC");
        assert_eq!(info.symbol.quote, "USDT");
        assert_eq!(info.base_precision, 5);
        assert_eq!(info.quote_precision, 1);
        assert_eq!(info.status, SymbolStatus::Trading);
        assert_eq!(info.min_qty, Some(dec!(0.00001)));
        assert_eq!(info.tick_size, Some(dec!(0.1)));
    }

    #[test]
    fn test_orderbook_raw_into_orderbook() {
        let raw = OkxOrderBookRaw {
            asks: vec![
                vec!["43000.1".into(), "0.5".into(), "0".into(), "3".into()],
            ],
            bids: vec![
                vec!["42999.9".into(), "1.2".into(), "0".into(), "5".into()],
            ],
            ts: "1700000000000".into(),
        };
        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(ExchangeId::Okx, sym);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks[0].price, dec!(43000.1));
        assert_eq!(ob.bids[0].price, dec!(42999.9));
    }

    #[test]
    fn test_ws_trade_data_into_trade() {
        let data = OkxWsTradeData {
            inst_id: "BTC-USDT".into(),
            trade_id: "999".into(),
            px: "44000.0".into(),
            sz: "0.5".into(),
            side: "sell".into(),
            ts: "1700000000000".into(),
        };
        let trade = data.into_trade(ExchangeId::Okx);
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.price, dec!(44000.0));
    }

    #[test]
    fn test_ws_book_msg_deserialize() {
        let json = r#"{
            "arg": {"channel": "books5", "instId": "BTC-USDT"},
            "data": [{
                "asks": [["43000.1", "0.5", "0", "3"]],
                "bids": [["42999.9", "1.2", "0", "5"]],
                "ts": "1700000000000"
            }]
        }"#;
        let msg: OkxWsBookMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.arg.channel, "books5");
        assert_eq!(msg.data[0].asks.len(), 1);
    }

    #[test]
    fn test_ws_trade_msg_deserialize() {
        let json = r#"{
            "arg": {"channel": "trades", "instId": "BTC-USDT"},
            "data": [{
                "instId": "BTC-USDT",
                "tradeId": "123",
                "px": "43000.0",
                "sz": "0.1",
                "side": "buy",
                "ts": "1700000000000"
            }]
        }"#;
        let msg: OkxWsTradeMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.data.len(), 1);
        assert_eq!(msg.data[0].side, "buy");
    }
}
