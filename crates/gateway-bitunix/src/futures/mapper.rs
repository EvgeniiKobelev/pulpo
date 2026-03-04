use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to a Bitunix futures symbol
/// (e.g. `Symbol::new("BTC","USDT")` -> `"BTCUSDT"`).
pub fn unified_to_bitunix(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Convert a Bitunix futures symbol to a unified Symbol
/// (e.g. `"BTCUSDT"` -> `Symbol::new("BTC","USDT")`).
pub fn bitunix_symbol_to_unified(raw: &str) -> Symbol {
    // Try known quote currencies
    for quote in &["USDT", "USDC", "BUSD", "TUSD", "DAI"] {
        if raw.ends_with(quote) && raw.len() > quote.len() {
            let base = &raw[..raw.len() - quote.len()];
            return Symbol::new(base, *quote);
        }
    }
    // Fallback: assume last 4 chars are quote
    let mid = raw.len().saturating_sub(4);
    Symbol::new(&raw[..mid], &raw[mid..])
}

/// Map a unified Interval to Bitunix REST kline interval string.
///
/// Bitunix REST supports: 1m, 5m, 15m, 30m, 1h, 2h, 4h, 6h, 8h, 12h, 1d, 3d, 1w, 1M
pub fn interval_to_bitunix(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1m", // not supported, fallback to 1m
        Interval::M1 => "1m",
        Interval::M3 => "5m", // 3m not supported in REST, fallback to 5m
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "1w",
    }
}

/// Map a unified Interval to Bitunix WS kline channel name.
///
/// Bitunix WS kline channels: market_kline_{1min,3min,5min,15min,30min,60min,2h,4h,6h,8h,12h,1day,3day,1week,1month}
pub fn interval_to_bitunix_ws(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 | Interval::M1 => "market_kline_1min",
        Interval::M3 => "market_kline_3min",
        Interval::M5 => "market_kline_5min",
        Interval::M15 => "market_kline_15min",
        Interval::M30 => "market_kline_30min",
        Interval::H1 => "market_kline_60min",
        Interval::H4 => "market_kline_4h",
        Interval::D1 => "market_kline_1day",
        Interval::W1 => "market_kline_1week",
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/futures/market/trading_pairs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitunixTradingPairRaw {
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub quote: String,
    #[serde(rename = "minTradeVolume", default)]
    pub min_trade_volume: Option<String>,
    #[serde(rename = "basePrecision", default)]
    pub base_precision: Option<u8>,
    #[serde(rename = "quotePrecision", default)]
    pub quote_precision: Option<u8>,
    #[serde(rename = "maxLeverage", default)]
    pub max_leverage: Option<u32>,
    #[serde(rename = "symbolStatus", default)]
    pub symbol_status: Option<String>,
}

impl BitunixTradingPairRaw {
    pub fn into_symbol_info(self) -> SymbolInfo {
        let base_prec = self.base_precision.unwrap_or(4);
        let quote_prec = self.quote_precision.unwrap_or(2);

        let min_qty = self
            .min_trade_volume
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());

        // Derive tick_size from quote precision: e.g. precision=1 -> tick_size=0.1
        let tick_size = if quote_prec > 0 {
            Decimal::new(1, quote_prec as u32)
        } else {
            Decimal::ONE
        };

        let status = match self.symbol_status.as_deref() {
            Some("OPEN") => SymbolStatus::Trading,
            Some("STOP") => SymbolStatus::Halted,
            Some("CANCEL_ONLY") => SymbolStatus::Halted,
            _ => SymbolStatus::Unknown,
        };

        SymbolInfo {
            symbol: Symbol::new(&self.base, &self.quote),
            raw_symbol: self.symbol,
            status,
            base_precision: base_prec,
            quote_precision: quote_prec,
            min_qty,
            min_notional: None,
            tick_size: Some(tick_size),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/futures/market/depth
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitunixDepthRaw {
    #[serde(default)]
    pub asks: Vec<Vec<String>>,
    #[serde(default)]
    pub bids: Vec<Vec<String>>,
}

impl BitunixDepthRaw {
    pub fn into_orderbook(self, symbol: &Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::BitunixFutures,
            symbol: symbol.clone(),
            bids: parse_levels_vec(&self.bids),
            asks: parse_levels_vec(&self.asks),
            timestamp_ms: now_ms(),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/futures/market/tickers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitunixTickerRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(rename = "markPrice", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "lastPrice", default)]
    pub last_price: Option<String>,
    #[serde(default)]
    pub open: Option<String>,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(rename = "quoteVol", default)]
    pub quote_vol: Option<String>,
    #[serde(rename = "baseVol", default)]
    pub base_vol: Option<String>,
    #[serde(default)]
    pub high: Option<String>,
    #[serde(default)]
    pub low: Option<String>,
}

impl BitunixTickerRaw {
    pub fn into_ticker(self, fallback_symbol: Option<&Symbol>) -> Ticker {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(bitunix_symbol_to_unified)
            .or_else(|| fallback_symbol.cloned())
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));

        let last_price = self
            .last_price
            .as_deref()
            .or(self.last.as_deref())
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        let volume_24h = self
            .base_vol
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        // Calculate price change % from open and last
        let open_price = self
            .open
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());
        let pct = open_price
            .filter(|o| !o.is_zero())
            .map(|o| (last_price - o) / o * Decimal::from(100));

        Ticker {
            exchange: ExchangeId::BitunixFutures,
            symbol,
            last_price,
            bid: None, // REST tickers endpoint does not include bid/ask
            ask: None,
            volume_24h,
            price_change_pct_24h: pct,
            timestamp_ms: now_ms(),
        }
    }

    pub fn into_mark_price(self, fallback_symbol: &Symbol) -> MarkPrice {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(bitunix_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        let mp = self
            .mark_price
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        let last = self
            .last_price
            .as_deref()
            .or(self.last.as_deref())
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(mp);

        MarkPrice {
            exchange: ExchangeId::BitunixFutures,
            symbol,
            mark_price: mp,
            index_price: last, // Bitunix doesn't provide separate index price in tickers
            timestamp_ms: now_ms(),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/futures/market/kline
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitunixKlineRaw {
    #[serde(default)]
    pub open: serde_json::Value,
    #[serde(default)]
    pub high: serde_json::Value,
    #[serde(default)]
    pub low: serde_json::Value,
    #[serde(default)]
    pub close: serde_json::Value,
    #[serde(rename = "quoteVol", default)]
    pub quote_vol: Option<String>,
    #[serde(rename = "baseVol", default)]
    pub base_vol: Option<String>,
    /// Timestamp (ms) — may come as number or string from Bitunix API.
    #[serde(default)]
    pub time: serde_json::Value,
    #[serde(rename = "type", default)]
    pub kline_type: Option<String>,
}

impl BitunixKlineRaw {
    pub fn into_candle(self, symbol: &Symbol, interval: Interval) -> Option<Candle> {
        let open = parse_decimal_value(&self.open)?;
        let high = parse_decimal_value(&self.high)?;
        let low = parse_decimal_value(&self.low)?;
        let close = parse_decimal_value(&self.close)?;
        let volume = self
            .base_vol
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let open_time_ms = parse_u64_value(&self.time)?;
        let close_time_ms = open_time_ms + interval.as_secs() * 1000;

        Some(Candle {
            exchange: ExchangeId::BitunixFutures,
            symbol: symbol.clone(),
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
}

// ---------------------------------------------------------------------------
// REST: /api/v1/futures/market/funding_rate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitunixFundingRateRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(rename = "markPrice", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "lastPrice", default)]
    pub last_price: Option<String>,
    #[serde(rename = "fundingRate", default)]
    pub funding_rate: Option<String>,
    #[serde(rename = "fundingInterval", default)]
    pub funding_interval: Option<u32>,
    #[serde(rename = "nextFundingTime", default)]
    pub next_funding_time: Option<String>,
}

impl BitunixFundingRateRaw {
    pub fn into_funding_rate(self, fallback_symbol: &Symbol) -> FundingRate {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(bitunix_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        FundingRate {
            exchange: ExchangeId::BitunixFutures,
            symbol,
            rate: self
                .funding_rate
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            next_funding_time_ms: self
                .next_funding_time
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0),
            timestamp_ms: now_ms(),
        }
    }
}

// ---------------------------------------------------------------------------
// WS: depth channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitunixWsDepthData {
    #[serde(default)]
    pub b: Vec<Vec<String>>,
    #[serde(default)]
    pub a: Vec<Vec<String>>,
}

impl BitunixWsDepthData {
    pub fn into_orderbook(self, symbol: &Symbol, ts: u64) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::BitunixFutures,
            symbol: symbol.clone(),
            bids: parse_levels_vec(&self.b),
            asks: parse_levels_vec(&self.a),
            timestamp_ms: ts,
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: trade channel
// ---------------------------------------------------------------------------

/// Bitunix WS trade data.
///
/// ```json
/// {"t":"2024-12-04T11:36:47.959Z","p":"27000.5","v":"0.001","s":"buy"}
/// ```
#[derive(Debug, Deserialize)]
pub struct BitunixWsTradeData {
    /// Timestamp (ISO 8601)
    #[serde(default)]
    pub t: Option<String>,
    /// Price
    #[serde(default)]
    pub p: Option<String>,
    /// Volume/quantity
    #[serde(default)]
    pub v: Option<String>,
    /// Side: "buy" or "sell"
    #[serde(default)]
    pub s: Option<String>,
}

impl BitunixWsTradeData {
    pub fn into_trade(self, symbol: &Symbol, outer_ts: u64) -> Option<Trade> {
        let side = match self.s.as_deref() {
            Some("sell") => Side::Sell,
            _ => Side::Buy,
        };
        Some(Trade {
            exchange: ExchangeId::BitunixFutures,
            symbol: symbol.clone(),
            price: Decimal::from_str(self.p.as_deref()?).ok()?,
            qty: Decimal::from_str(self.v.as_deref()?).ok()?,
            side,
            timestamp_ms: outer_ts,
            trade_id: None,
        })
    }
}

// ---------------------------------------------------------------------------
// WS: kline channel
// ---------------------------------------------------------------------------

/// Bitunix WS kline data.
///
/// ```json
/// {"o":"0.0010","c":"0.0020","h":"0.0025","l":"0.0015","b":"1.01","q":"1.09"}
/// ```
#[derive(Debug, Deserialize)]
pub struct BitunixWsKlineData {
    /// Open price
    #[serde(default)]
    pub o: Option<String>,
    /// High price
    #[serde(default)]
    pub h: Option<String>,
    /// Low price
    #[serde(default)]
    pub l: Option<String>,
    /// Close price
    #[serde(default)]
    pub c: Option<String>,
    /// Base volume
    #[serde(default)]
    pub b: Option<String>,
    /// Quote volume
    #[serde(default)]
    pub q: Option<String>,
}

impl BitunixWsKlineData {
    pub fn into_candle(self, symbol: &Symbol, interval: Interval, ts: u64) -> Option<Candle> {
        let interval_ms = interval.as_secs() * 1000;
        // Align open_time to interval boundary
        let open_time_ms = ts - (ts % interval_ms);
        let close_time_ms = open_time_ms + interval_ms;

        Some(Candle {
            exchange: ExchangeId::BitunixFutures,
            symbol: symbol.clone(),
            open: Decimal::from_str(self.o.as_deref()?).ok()?,
            high: Decimal::from_str(self.h.as_deref()?).ok()?,
            low: Decimal::from_str(self.l.as_deref()?).ok()?,
            close: Decimal::from_str(self.c.as_deref()?).ok()?,
            volume: self
                .b
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            open_time_ms,
            close_time_ms,
            is_closed: false,
        })
    }
}

// ---------------------------------------------------------------------------
// WS: price channel (mark price + index price + funding rate)
// ---------------------------------------------------------------------------

/// Bitunix WS market price data.
///
/// ```json
/// {"ip":"0.0010","mp":"10000","fr":"0.013461","ft":"2024-12-04T11:00:00Z","nft":"..."}
/// ```
#[derive(Debug, Deserialize)]
pub struct BitunixWsMarkPriceData {
    /// Mark price
    #[serde(default)]
    pub mp: Option<String>,
    /// Index price
    #[serde(default)]
    pub ip: Option<String>,
    /// Funding rate
    #[serde(default)]
    pub fr: Option<String>,
    /// Funding time
    #[serde(default)]
    pub ft: Option<String>,
    /// Next funding time
    #[serde(default)]
    pub nft: Option<String>,
}

impl BitunixWsMarkPriceData {
    pub fn into_mark_price(self, symbol: &Symbol, ts: u64) -> MarkPrice {
        let mp = self
            .mp
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let ip = self
            .ip
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(mp);

        MarkPrice {
            exchange: ExchangeId::BitunixFutures,
            symbol: symbol.clone(),
            mark_price: mp,
            index_price: ip,
            timestamp_ms: ts,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse price/qty pairs from Bitunix format `[["price","qty"], ...]`.
fn parse_levels_vec(raw: &[Vec<String>]) -> Vec<Level> {
    raw.iter()
        .filter_map(|pair| {
            if pair.len() < 2 {
                return None;
            }
            let price = Decimal::from_str(&pair[0]).ok()?;
            let qty = Decimal::from_str(&pair[1]).ok()?;
            Some(Level::new(price, qty))
        })
        .collect()
}

/// Parse a serde_json::Value to u64 (handles both string and number).
fn parse_u64_value(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        Some(n)
    } else if let Some(s) = v.as_str() {
        s.parse().ok()
    } else {
        None
    }
}

/// Parse a serde_json::Value to Decimal (handles both string and number).
pub fn parse_decimal_value(v: &serde_json::Value) -> Option<Decimal> {
    if let Some(s) = v.as_str() {
        Decimal::from_str(s).ok()
    } else if let Some(n) = v.as_f64() {
        Decimal::try_from(n).ok()
    } else {
        None
    }
}

/// Current time in milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_unified_to_bitunix() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_bitunix(&sym), "BTCUSDT");
    }

    #[test]
    fn test_bitunix_symbol_to_unified() {
        let sym = bitunix_symbol_to_unified("BTCUSDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = bitunix_symbol_to_unified("ETHUSDT");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDT");

        let sym3 = bitunix_symbol_to_unified("SOLUSDC");
        assert_eq!(sym3.base, "SOL");
        assert_eq!(sym3.quote, "USDC");
    }

    #[test]
    fn test_interval_to_bitunix() {
        assert_eq!(interval_to_bitunix(Interval::M1), "1m");
        assert_eq!(interval_to_bitunix(Interval::M5), "5m");
        assert_eq!(interval_to_bitunix(Interval::H1), "1h");
        assert_eq!(interval_to_bitunix(Interval::H4), "4h");
        assert_eq!(interval_to_bitunix(Interval::D1), "1d");
        assert_eq!(interval_to_bitunix(Interval::W1), "1w");
    }

    #[test]
    fn test_interval_to_bitunix_ws() {
        assert_eq!(interval_to_bitunix_ws(Interval::M1), "market_kline_1min");
        assert_eq!(interval_to_bitunix_ws(Interval::M3), "market_kline_3min");
        assert_eq!(interval_to_bitunix_ws(Interval::H1), "market_kline_60min");
        assert_eq!(interval_to_bitunix_ws(Interval::D1), "market_kline_1day");
    }

    #[test]
    fn test_depth_into_orderbook() {
        let raw: BitunixDepthRaw = serde_json::from_str(
            r#"{
                "asks": [["50001.5", "0.5"], ["50002.0", "1.0"]],
                "bids": [["50000.0", "2.0"], ["49999.5", "1.5"]]
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(&sym);
        assert_eq!(ob.exchange, ExchangeId::BitunixFutures);
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 2);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.bids[0].qty, dec!(2.0));
        assert_eq!(ob.asks[0].price, dec!(50001.5));
    }

    #[test]
    fn test_ticker_into_ticker() {
        let raw: BitunixTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "markPrice": "50100.0",
                "lastPrice": "50000.0",
                "open": "49000.0",
                "last": "50000.0",
                "quoteVol": "617283900.0",
                "baseVol": "12345.678",
                "high": "51000.0",
                "low": "48000.0"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker(None);
        assert_eq!(ticker.exchange, ExchangeId::BitunixFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000.0));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert!(ticker.price_change_pct_24h.is_some());
    }

    #[test]
    fn test_kline_into_candle() {
        let raw: BitunixKlineRaw = serde_json::from_str(
            r#"{
                "open": 60000,
                "high": 60001,
                "close": 60000,
                "low": 59989.2,
                "time": 1700000000000,
                "quoteVol": "1",
                "baseVol": "60000",
                "type": "LAST_PRICE"
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let candle = raw.into_candle(&sym, Interval::M1).unwrap();
        assert_eq!(candle.exchange, ExchangeId::BitunixFutures);
        assert_eq!(candle.open, dec!(60000));
        assert_eq!(candle.high, dec!(60001));
        assert_eq!(candle.close, dec!(60000));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000);
    }

    #[test]
    fn test_funding_rate_into_funding_rate() {
        let raw: BitunixFundingRateRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "markPrice": "60000",
                "lastPrice": "60001",
                "fundingRate": "0.0005",
                "fundingInterval": 8,
                "nextFundingTime": "1770710400000"
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let fr = raw.into_funding_rate(&sym);
        assert_eq!(fr.exchange, ExchangeId::BitunixFutures);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(fr.rate, dec!(0.0005));
        assert_eq!(fr.next_funding_time_ms, 1770710400000);
    }

    #[test]
    fn test_ws_trade_into_trade() {
        let raw: BitunixWsTradeData = serde_json::from_str(
            r#"{
                "t": "2024-12-04T11:36:47.959Z",
                "p": "50000.5",
                "v": "0.1",
                "s": "buy"
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym, 1700000000000).unwrap();
        assert_eq!(trade.exchange, ExchangeId::BitunixFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
        assert_eq!(trade.qty, dec!(0.1));
    }

    #[test]
    fn test_ws_trade_sell_side() {
        let raw: BitunixWsTradeData = serde_json::from_str(
            r#"{"p": "50000.5", "v": "0.5", "s": "sell"}"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym, 1700000000000).unwrap();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ws_depth_into_orderbook() {
        let raw: BitunixWsDepthData = serde_json::from_str(
            r#"{
                "b": [["50000.0", "1.5"]],
                "a": [["50001.0", "2.0"]]
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(&sym, 1700000000000);
        assert_eq!(ob.exchange, ExchangeId::BitunixFutures);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.asks[0].price, dec!(50001.0));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_kline_into_candle() {
        let raw: BitunixWsKlineData = serde_json::from_str(
            r#"{
                "o": "50000.0",
                "h": "51000.0",
                "l": "49000.0",
                "c": "50500.0",
                "b": "100.5",
                "q": "5050000.0"
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let candle = raw.into_candle(&sym, Interval::M1, 1700000060000).unwrap();
        assert_eq!(candle.exchange, ExchangeId::BitunixFutures);
        assert_eq!(candle.open, dec!(50000.0));
        assert_eq!(candle.close, dec!(50500.0));
        assert_eq!(candle.volume, dec!(100.5));
    }

    #[test]
    fn test_ws_mark_price_into_mark_price() {
        let raw: BitunixWsMarkPriceData = serde_json::from_str(
            r#"{
                "mp": "50010.0",
                "ip": "50005.0",
                "fr": "0.0001",
                "ft": "2024-12-04T11:00:00Z",
                "nft": "2024-12-04T12:00:00Z"
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let mp = raw.into_mark_price(&sym, 1700000000000);
        assert_eq!(mp.mark_price, dec!(50010.0));
        assert_eq!(mp.index_price, dec!(50005.0));
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_parse_levels_vec() {
        let raw = vec![
            vec!["100.50".to_string(), "1.5".to_string()],
            vec!["99.00".to_string(), "2.0".to_string()],
        ];
        let levels = parse_levels_vec(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(100.50));
        assert_eq!(levels[0].qty, dec!(1.5));
    }

    #[test]
    fn test_trading_pair_into_symbol_info() {
        let raw: BitunixTradingPairRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "base": "BTC",
                "quote": "USDT",
                "minTradeVolume": "0.0001",
                "basePrecision": 4,
                "quotePrecision": 1,
                "maxLeverage": 125,
                "symbolStatus": "OPEN"
            }"#,
        )
        .unwrap();

        let info = raw.into_symbol_info();
        assert_eq!(info.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.raw_symbol, "BTCUSDT");
        assert_eq!(info.base_precision, 4);
        assert_eq!(info.quote_precision, 1);
        assert_eq!(info.status, SymbolStatus::Trading);
        assert_eq!(info.min_qty, Some(dec!(0.0001)));
    }
}
