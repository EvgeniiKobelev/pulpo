use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to a Hyperliquid coin name (e.g. `Symbol::new("BTC","USDC")` → `"BTC"`).
///
/// Hyperliquid perpetuals use the base asset name only — all contracts settle in USDC.
pub fn unified_to_hl(symbol: &Symbol) -> String {
    symbol.base.clone()
}

/// Convert a Hyperliquid coin name to a unified Symbol (e.g. `"BTC"` → `Symbol::new("BTC","USDC")`).
///
/// All Hyperliquid perps are quoted in USDC.
pub fn hl_coin_to_unified(coin: &str) -> Symbol {
    Symbol::new(coin, "USDC")
}

/// Map a unified Interval to the Hyperliquid candle interval string.
pub fn interval_to_hl(interval: Interval) -> &'static str {
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
// REST: meta
// ---------------------------------------------------------------------------

/// Response from `{"type": "meta"}`.
#[derive(Debug, Deserialize)]
pub struct HlMetaRaw {
    pub universe: Vec<HlAssetRaw>,
}

#[derive(Debug, Deserialize)]
pub struct HlAssetRaw {
    pub name: String,
    #[serde(rename = "szDecimals")]
    pub sz_decimals: u8,
    #[serde(rename = "maxLeverage")]
    pub max_leverage: u32,
}

impl HlMetaRaw {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .universe
            .into_iter()
            .map(|a| {
                let symbol = hl_coin_to_unified(&a.name);
                SymbolInfo {
                    raw_symbol: a.name,
                    symbol,
                    status: SymbolStatus::Trading,
                    base_precision: a.sz_decimals,
                    quote_precision: 6, // USDC has 6 decimals
                    min_qty: None,
                    min_notional: None,
                    tick_size: None,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::Hyperliquid,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: metaAndAssetCtxs
// ---------------------------------------------------------------------------

/// Asset context from `metaAndAssetCtxs` response.
#[derive(Debug, Deserialize)]
pub struct HlAssetCtxRaw {
    #[serde(rename = "markPx")]
    pub mark_px: String,
    #[serde(rename = "midPx")]
    pub mid_px: Option<String>,
    #[serde(rename = "oraclePx")]
    pub oracle_px: String,
    pub funding: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
    #[serde(rename = "dayNtlVlm")]
    pub day_ntl_vlm: String,
    #[serde(rename = "prevDayPx")]
    pub prev_day_px: String,
    #[serde(rename = "impactPxs", default)]
    pub impact_pxs: Option<[String; 2]>,
}

impl HlAssetCtxRaw {
    pub fn into_ticker(self, coin: &str) -> Ticker {
        let symbol = hl_coin_to_unified(coin);
        let mark = Decimal::from_str(&self.mark_px).unwrap_or_default();
        let prev = Decimal::from_str(&self.prev_day_px).unwrap_or_default();

        let pct = if !prev.is_zero() {
            Some(((mark - prev) / prev) * Decimal::from(100))
        } else {
            None
        };

        let (bid, ask) = match &self.impact_pxs {
            Some([b, a]) => (
                Decimal::from_str(b).ok(),
                Decimal::from_str(a).ok(),
            ),
            None => (None, None),
        };

        Ticker {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            last_price: mark,
            bid,
            ask,
            volume_24h: Decimal::from_str(&self.day_ntl_vlm).unwrap_or_default(),
            price_change_pct_24h: pct,
            timestamp_ms: 0,
        }
    }

    pub fn into_mark_price(self, coin: &str) -> MarkPrice {
        let symbol = hl_coin_to_unified(coin);
        MarkPrice {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            mark_price: Decimal::from_str(&self.mark_px).unwrap_or_default(),
            index_price: Decimal::from_str(&self.oracle_px).unwrap_or_default(),
            timestamp_ms: 0,
        }
    }

    pub fn into_funding_rate(self, coin: &str) -> FundingRate {
        let symbol = hl_coin_to_unified(coin);
        FundingRate {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            rate: Decimal::from_str(&self.funding).unwrap_or_default(),
            next_funding_time_ms: 0,
            timestamp_ms: 0,
        }
    }

    pub fn into_open_interest(self, coin: &str) -> OpenInterest {
        let symbol = hl_coin_to_unified(coin);
        let oi = Decimal::from_str(&self.open_interest).unwrap_or_default();
        let mark = Decimal::from_str(&self.mark_px).unwrap_or_default();
        OpenInterest {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            open_interest: oi,
            open_interest_value: oi * mark,
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: l2Book
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HlL2BookLevel {
    pub px: String,
    pub sz: String,
    pub n: u32,
}

#[derive(Debug, Deserialize)]
pub struct HlL2BookRaw {
    pub coin: String,
    pub time: u64,
    pub levels: Vec<Vec<HlL2BookLevel>>,
}

impl HlL2BookRaw {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = hl_coin_to_unified(&self.coin);
        let bids = self
            .levels
            .first()
            .map(|lvls| parse_hl_levels(lvls))
            .unwrap_or_default();
        let asks = self
            .levels
            .get(1)
            .map(|lvls| parse_hl_levels(lvls))
            .unwrap_or_default();

        OrderBook {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            bids,
            asks,
            timestamp_ms: self.time,
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: candleSnapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HlCandleRaw {
    /// Open time (ms)
    pub t: u64,
    /// Close time (ms)
    #[serde(rename = "T")]
    pub close_time: u64,
    /// Coin name
    pub s: String,
    /// Interval
    pub i: String,
    /// Open
    pub o: String,
    /// Close
    pub c: String,
    /// High
    pub h: String,
    /// Low
    pub l: String,
    /// Volume (base)
    pub v: String,
    /// Number of trades
    pub n: u64,
}

impl HlCandleRaw {
    pub fn into_candle(self) -> Candle {
        let symbol = hl_coin_to_unified(&self.s);
        Candle {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            open: Decimal::from_str(&self.o).unwrap_or_default(),
            high: Decimal::from_str(&self.h).unwrap_or_default(),
            low: Decimal::from_str(&self.l).unwrap_or_default(),
            close: Decimal::from_str(&self.c).unwrap_or_default(),
            volume: Decimal::from_str(&self.v).unwrap_or_default(),
            open_time_ms: self.t,
            close_time_ms: self.close_time,
            is_closed: true,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: l2Book
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HlWsL2BookMsg {
    pub channel: String,
    pub data: HlWsL2BookData,
}

#[derive(Debug, Deserialize)]
pub struct HlWsL2BookData {
    pub coin: String,
    pub time: u64,
    pub levels: Vec<Vec<HlL2BookLevel>>,
}

impl HlWsL2BookData {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = hl_coin_to_unified(&self.coin);
        let bids = self
            .levels
            .first()
            .map(|lvls| parse_hl_levels(lvls))
            .unwrap_or_default();
        let asks = self
            .levels
            .get(1)
            .map(|lvls| parse_hl_levels(lvls))
            .unwrap_or_default();

        OrderBook {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            bids,
            asks,
            timestamp_ms: self.time,
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: trades
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HlWsTradesMsg {
    pub channel: String,
    pub data: Vec<HlWsTradeData>,
}

#[derive(Debug, Deserialize)]
pub struct HlWsTradeData {
    pub coin: String,
    pub side: String,
    pub px: String,
    pub sz: String,
    pub time: u64,
    pub tid: u64,
    pub hash: String,
}

impl HlWsTradeData {
    pub fn into_trade(self) -> Trade {
        let symbol = hl_coin_to_unified(&self.coin);
        let side = match self.side.as_str() {
            "B" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            price: Decimal::from_str(&self.px).unwrap_or_default(),
            qty: Decimal::from_str(&self.sz).unwrap_or_default(),
            side,
            timestamp_ms: self.time,
            trade_id: Some(self.tid.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// WS: candle
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HlWsCandleMsg {
    pub channel: String,
    pub data: HlWsCandleData,
}

#[derive(Debug, Deserialize)]
pub struct HlWsCandleData {
    pub t: u64,
    #[serde(rename = "T")]
    pub close_time: u64,
    pub s: String,
    pub i: String,
    pub o: String,
    pub c: String,
    pub h: String,
    pub l: String,
    pub v: String,
    pub n: u64,
}

impl HlWsCandleData {
    pub fn into_candle(self) -> Candle {
        let symbol = hl_coin_to_unified(&self.s);
        Candle {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            open: Decimal::from_str(&self.o).unwrap_or_default(),
            high: Decimal::from_str(&self.h).unwrap_or_default(),
            low: Decimal::from_str(&self.l).unwrap_or_default(),
            close: Decimal::from_str(&self.c).unwrap_or_default(),
            volume: Decimal::from_str(&self.v).unwrap_or_default(),
            open_time_ms: self.t,
            close_time_ms: self.close_time,
            is_closed: false,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: activeAssetCtx (mark price)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HlWsActiveAssetCtxMsg {
    pub channel: String,
    pub data: HlWsActiveAssetCtxData,
}

#[derive(Debug, Deserialize)]
pub struct HlWsActiveAssetCtxData {
    pub coin: String,
    pub ctx: HlWsAssetCtx,
}

#[derive(Debug, Deserialize)]
pub struct HlWsAssetCtx {
    #[serde(rename = "markPx")]
    pub mark_px: String,
    #[serde(rename = "oraclePx")]
    pub oracle_px: String,
    pub funding: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
}

impl HlWsActiveAssetCtxData {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = hl_coin_to_unified(&self.coin);
        MarkPrice {
            exchange: ExchangeId::Hyperliquid,
            symbol,
            mark_price: Decimal::from_str(&self.ctx.mark_px).unwrap_or_default(),
            index_price: Decimal::from_str(&self.ctx.oracle_px).unwrap_or_default(),
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_hl_levels(raw: &[HlL2BookLevel]) -> Vec<Level> {
    raw.iter()
        .filter_map(|lvl| {
            let price = Decimal::from_str(&lvl.px).ok()?;
            let qty = Decimal::from_str(&lvl.sz).ok()?;
            Some(Level::new(price, qty))
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
    fn test_unified_to_hl() {
        let sym = Symbol::new("BTC", "USDC");
        assert_eq!(unified_to_hl(&sym), "BTC");
    }

    #[test]
    fn test_hl_coin_to_unified() {
        let sym = hl_coin_to_unified("BTC");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDC");
    }

    #[test]
    fn test_interval_to_hl() {
        assert_eq!(interval_to_hl(Interval::M1), "1m");
        assert_eq!(interval_to_hl(Interval::H1), "1h");
        assert_eq!(interval_to_hl(Interval::D1), "1d");
    }

    #[test]
    fn test_meta_into_exchange_info() {
        let raw: HlMetaRaw = serde_json::from_str(
            r#"{
                "universe": [
                    {"name": "BTC", "szDecimals": 5, "maxLeverage": 50},
                    {"name": "ETH", "szDecimals": 4, "maxLeverage": 50}
                ]
            }"#,
        )
        .unwrap();

        let info = raw.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::Hyperliquid);
        assert_eq!(info.symbols.len(), 2);
        assert_eq!(info.symbols[0].symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(info.symbols[0].raw_symbol, "BTC");
        assert_eq!(info.symbols[0].base_precision, 5);
        assert_eq!(info.symbols[1].symbol, Symbol::new("ETH", "USDC"));
    }

    #[test]
    fn test_asset_ctx_into_ticker() {
        let raw: HlAssetCtxRaw = serde_json::from_str(
            r#"{
                "markPx": "50000.0",
                "midPx": "50000.5",
                "oraclePx": "49990.0",
                "funding": "0.0001",
                "openInterest": "1234.5",
                "dayNtlVlm": "500000000.0",
                "prevDayPx": "49000.0",
                "impactPxs": ["49999.0", "50001.0"]
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker("BTC");
        assert_eq!(ticker.exchange, ExchangeId::Hyperliquid);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(ticker.last_price, dec!(50000.0));
        assert_eq!(ticker.bid, Some(dec!(49999.0)));
        assert_eq!(ticker.ask, Some(dec!(50001.0)));
        assert_eq!(ticker.volume_24h, dec!(500000000.0));
        assert!(ticker.price_change_pct_24h.is_some());
    }

    #[test]
    fn test_asset_ctx_into_mark_price() {
        let raw: HlAssetCtxRaw = serde_json::from_str(
            r#"{
                "markPx": "2000.50",
                "midPx": "2000.25",
                "oraclePx": "2000.00",
                "funding": "0.0002",
                "openInterest": "5000.0",
                "dayNtlVlm": "100000000.0",
                "prevDayPx": "1990.0"
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price("ETH");
        assert_eq!(mp.exchange, ExchangeId::Hyperliquid);
        assert_eq!(mp.symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(mp.mark_price, dec!(2000.50));
        assert_eq!(mp.index_price, dec!(2000.00));
    }

    #[test]
    fn test_asset_ctx_into_funding_rate() {
        let raw: HlAssetCtxRaw = serde_json::from_str(
            r#"{
                "markPx": "50000.0",
                "midPx": "50000.5",
                "oraclePx": "49990.0",
                "funding": "0.000123",
                "openInterest": "1234.5",
                "dayNtlVlm": "500000000.0",
                "prevDayPx": "49000.0"
            }"#,
        )
        .unwrap();

        let fr = raw.into_funding_rate("BTC");
        assert_eq!(fr.exchange, ExchangeId::Hyperliquid);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(fr.rate, dec!(0.000123));
    }

    #[test]
    fn test_asset_ctx_into_open_interest() {
        let raw: HlAssetCtxRaw = serde_json::from_str(
            r#"{
                "markPx": "50000.0",
                "midPx": "50000.5",
                "oraclePx": "49990.0",
                "funding": "0.0001",
                "openInterest": "100.5",
                "dayNtlVlm": "500000000.0",
                "prevDayPx": "49000.0"
            }"#,
        )
        .unwrap();

        let oi = raw.into_open_interest("BTC");
        assert_eq!(oi.exchange, ExchangeId::Hyperliquid);
        assert_eq!(oi.open_interest, dec!(100.5));
        assert_eq!(oi.open_interest_value, dec!(100.5) * dec!(50000.0));
    }

    #[test]
    fn test_l2book_into_orderbook() {
        let raw: HlL2BookRaw = serde_json::from_str(
            r#"{
                "coin": "BTC",
                "time": 1700000000000,
                "levels": [
                    [{"px": "50000.0", "sz": "1.5", "n": 3}],
                    [{"px": "50001.0", "sz": "2.0", "n": 5}]
                ]
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::Hyperliquid);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.bids[0].qty, dec!(1.5));
        assert_eq!(ob.asks[0].price, dec!(50001.0));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_candle_raw_into_candle() {
        let raw: HlCandleRaw = serde_json::from_str(
            r#"{
                "t": 1700000000000,
                "T": 1700000060000,
                "s": "BTC",
                "i": "1m",
                "o": "50000.0",
                "c": "50100.0",
                "h": "50200.0",
                "l": "49900.0",
                "v": "123.45",
                "n": 500
            }"#,
        )
        .unwrap();

        let candle = raw.into_candle();
        assert_eq!(candle.exchange, ExchangeId::Hyperliquid);
        assert_eq!(candle.symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(candle.open, dec!(50000.0));
        assert_eq!(candle.close, dec!(50100.0));
        assert_eq!(candle.high, dec!(50200.0));
        assert_eq!(candle.low, dec!(49900.0));
        assert_eq!(candle.volume, dec!(123.45));
        assert!(candle.is_closed);
    }

    #[test]
    fn test_ws_l2book_into_orderbook() {
        let raw: HlWsL2BookMsg = serde_json::from_str(
            r#"{
                "channel": "l2Book",
                "data": {
                    "coin": "ETH",
                    "time": 1700000000000,
                    "levels": [
                        [{"px": "2000.0", "sz": "10.0", "n": 2}, {"px": "1999.0", "sz": "20.0", "n": 4}],
                        [{"px": "2001.0", "sz": "5.0", "n": 1}]
                    ]
                }
            }"#,
        )
        .unwrap();

        let ob = raw.data.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::Hyperliquid);
        assert_eq!(ob.symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(2000.0));
    }

    #[test]
    fn test_ws_trade_into_trade() {
        let raw: HlWsTradesMsg = serde_json::from_str(
            r#"{
                "channel": "trades",
                "data": [{
                    "coin": "BTC",
                    "side": "B",
                    "px": "50123.45",
                    "sz": "0.5",
                    "time": 1700000000000,
                    "tid": 12345,
                    "hash": "0xabc"
                }]
            }"#,
        )
        .unwrap();

        let trade = raw.data.into_iter().next().unwrap().into_trade();
        assert_eq!(trade.exchange, ExchangeId::Hyperliquid);
        assert_eq!(trade.symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50123.45));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.trade_id, Some("12345".to_string()));
    }

    #[test]
    fn test_ws_trade_sell_side() {
        let raw: HlWsTradeData = serde_json::from_str(
            r#"{
                "coin": "ETH",
                "side": "A",
                "px": "2000.0",
                "sz": "1.0",
                "time": 1700000000000,
                "tid": 99,
                "hash": "0xdef"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ws_candle_into_candle() {
        let raw: HlWsCandleMsg = serde_json::from_str(
            r#"{
                "channel": "candle",
                "data": {
                    "t": 1700000000000,
                    "T": 1700000060000,
                    "s": "BTC",
                    "i": "1m",
                    "o": "50000.0",
                    "c": "50050.0",
                    "h": "50100.0",
                    "l": "49950.0",
                    "v": "10.5",
                    "n": 42
                }
            }"#,
        )
        .unwrap();

        let candle = raw.data.into_candle();
        assert_eq!(candle.exchange, ExchangeId::Hyperliquid);
        assert_eq!(candle.open, dec!(50000.0));
        assert!(!candle.is_closed);
    }

    #[test]
    fn test_ws_active_asset_ctx_into_mark_price() {
        let raw: HlWsActiveAssetCtxMsg = serde_json::from_str(
            r#"{
                "channel": "activeAssetCtx",
                "data": {
                    "coin": "BTC",
                    "ctx": {
                        "markPx": "50123.0",
                        "oraclePx": "50100.0",
                        "funding": "0.0001",
                        "openInterest": "5000.0"
                    }
                }
            }"#,
        )
        .unwrap();

        let mp = raw.data.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::Hyperliquid);
        assert_eq!(mp.symbol, Symbol::new("BTC", "USDC"));
        assert_eq!(mp.mark_price, dec!(50123.0));
        assert_eq!(mp.index_price, dec!(50100.0));
    }

    #[test]
    fn test_parse_hl_levels() {
        let raw = vec![
            HlL2BookLevel { px: "100.50".to_string(), sz: "1.5".to_string(), n: 1 },
            HlL2BookLevel { px: "99.00".to_string(), sz: "2.0".to_string(), n: 2 },
        ];
        let levels = parse_hl_levels(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(100.50));
        assert_eq!(levels[0].qty, dec!(1.5));
    }

    #[test]
    fn test_parse_hl_levels_skips_invalid() {
        let raw = vec![
            HlL2BookLevel { px: "bad".to_string(), sz: "1.0".to_string(), n: 1 },
            HlL2BookLevel { px: "50.00".to_string(), sz: "3.0".to_string(), n: 1 },
        ];
        let levels = parse_hl_levels(&raw);
        assert_eq!(levels.len(), 1);
    }

    #[test]
    fn test_ticker_no_impact_pxs() {
        let raw: HlAssetCtxRaw = serde_json::from_str(
            r#"{
                "markPx": "50000.0",
                "midPx": "50000.5",
                "oraclePx": "49990.0",
                "funding": "0.0001",
                "openInterest": "1234.5",
                "dayNtlVlm": "500000000.0",
                "prevDayPx": "49000.0"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker("BTC");
        assert_eq!(ticker.bid, None);
        assert_eq!(ticker.ask, None);
    }
}
