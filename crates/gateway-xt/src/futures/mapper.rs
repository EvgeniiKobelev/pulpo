use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to an XT futures symbol
/// (e.g. `Symbol::new("BTC","USDT")` -> `"btc_usdt"`).
pub fn unified_to_xt(symbol: &Symbol) -> String {
    format!("{}_{}", symbol.base.to_lowercase(), symbol.quote.to_lowercase())
}

/// Convert an XT futures symbol to a unified Symbol
/// (e.g. `"btc_usdt"` -> `Symbol::new("BTC","USDT")`).
pub fn xt_symbol_to_unified(raw: &str) -> Symbol {
    if let Some((base, quote)) = raw.split_once('_') {
        Symbol::new(base, quote)
    } else {
        // Fallback: try known quote currencies
        let upper = raw.to_uppercase();
        for quote in &["USDT", "USDC", "BUSD", "TUSD", "DAI"] {
            if upper.ends_with(quote) && upper.len() > quote.len() {
                let base = &upper[..upper.len() - quote.len()];
                return Symbol::new(base, *quote);
            }
        }
        Symbol::new(raw, "UNKNOWN")
    }
}

/// Map a unified Interval to XT REST kline interval string.
///
/// XT supports: 1m, 5m, 15m, 30m, 1h, 4h, 1d, 1w
pub fn interval_to_xt(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1m",
        Interval::M1 => "1m",
        Interval::M3 => "5m", // 3m not supported, fallback to 5m
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "1w",
    }
}

/// Map a unified Interval to XT WS kline interval string.
pub fn interval_to_xt_ws(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 | Interval::M1 => "1m",
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
// REST: /future/market/v1/public/symbol/list
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtSymbolRaw {
    #[serde(default)]
    pub symbol: String,
    #[serde(rename = "baseCoin", default)]
    pub base_coin: String,
    #[serde(rename = "quoteCoin", default)]
    pub quote_coin: String,
    #[serde(rename = "baseCoinDisplayPrecision", default)]
    pub base_coin_display_precision: Option<u8>,
    #[serde(rename = "pricePrecision", default)]
    pub price_precision: Option<u8>,
    #[serde(rename = "quantityPrecision", default)]
    pub quantity_precision: Option<u8>,
    #[serde(rename = "minQty", default)]
    pub min_qty: Option<String>,
    #[serde(rename = "minNotional", default)]
    pub min_notional: Option<String>,
    #[serde(rename = "minStepPrice", default)]
    pub min_step_price: Option<String>,
    #[serde(rename = "contractType", default)]
    pub contract_type: Option<String>,
    /// state: 0 = trading
    #[serde(default)]
    pub state: Option<i32>,
    #[serde(rename = "tradeSwitch", default)]
    pub trade_switch: Option<bool>,
}

impl XtSymbolRaw {
    pub fn into_symbol_info(self) -> SymbolInfo {
        let base_prec = self.base_coin_display_precision.unwrap_or(4);
        let price_prec = self.price_precision.unwrap_or(2);

        let min_qty = self
            .min_qty
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());

        let min_notional = self
            .min_notional
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());

        let tick_size = self
            .min_step_price
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .or_else(|| {
                if price_prec > 0 {
                    Some(Decimal::new(1, price_prec as u32))
                } else {
                    Some(Decimal::ONE)
                }
            });

        let status = match (self.state, self.trade_switch) {
            (Some(0), Some(true)) => SymbolStatus::Trading,
            (Some(0), _) => SymbolStatus::Trading,
            _ => SymbolStatus::Unknown,
        };

        SymbolInfo {
            symbol: Symbol::new(&self.base_coin, &self.quote_coin),
            raw_symbol: self.symbol,
            status,
            base_precision: base_prec,
            quote_precision: price_prec,
            min_qty,
            min_notional,
            tick_size,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/q/depth
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtDepthRaw {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Asks: [[price, qty], ...]
    #[serde(default)]
    pub a: Vec<Vec<String>>,
    /// Bids: [[price, qty], ...]
    #[serde(default)]
    pub b: Vec<Vec<String>>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtDepthRaw {
    pub fn into_orderbook(self, symbol: &Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::XtFutures,
            symbol: symbol.clone(),
            bids: parse_levels_vec(&self.b),
            asks: parse_levels_vec(&self.a),
            timestamp_ms: self.t.unwrap_or_else(now_ms),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/q/agg-ticker(s)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtAggTickerRaw {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Open price 24h
    #[serde(default)]
    pub o: Option<String>,
    /// Close/last price
    #[serde(default)]
    pub c: Option<String>,
    /// High 24h
    #[serde(default)]
    pub h: Option<String>,
    /// Low 24h
    #[serde(default)]
    pub l: Option<String>,
    /// Volume 24h (base)
    #[serde(default)]
    pub a: Option<String>,
    /// Turnover 24h (quote)
    #[serde(default)]
    pub v: Option<String>,
    /// Change percentage
    #[serde(default)]
    pub r: Option<String>,
    /// Index price
    #[serde(default)]
    pub i: Option<String>,
    /// Mark price
    #[serde(default)]
    pub m: Option<String>,
    /// Best bid price
    #[serde(default)]
    pub bp: Option<String>,
    /// Best ask price
    #[serde(default)]
    pub ap: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtAggTickerRaw {
    pub fn into_ticker(self, fallback_symbol: Option<&Symbol>) -> Ticker {
        let symbol = self
            .s
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(xt_symbol_to_unified)
            .or_else(|| fallback_symbol.cloned())
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));

        let last_price = self
            .c
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        let volume_24h = self
            .a
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        let pct = self
            .r
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .map(|r| r * Decimal::from(100));

        let bid = self.bp.as_deref().and_then(|s| Decimal::from_str(s).ok());
        let ask = self.ap.as_deref().and_then(|s| Decimal::from_str(s).ok());

        Ticker {
            exchange: ExchangeId::XtFutures,
            symbol,
            last_price,
            bid,
            ask,
            volume_24h,
            price_change_pct_24h: pct,
            timestamp_ms: self.t.unwrap_or_else(now_ms),
        }
    }

    pub fn into_mark_price(self, fallback_symbol: &Symbol) -> MarkPrice {
        let symbol = self
            .s
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(xt_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        let mp = self
            .m
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        let ip = self
            .i
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(mp);

        MarkPrice {
            exchange: ExchangeId::XtFutures,
            symbol,
            mark_price: mp,
            index_price: ip,
            timestamp_ms: self.t.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/q/ticker (simple ticker)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtTickerRaw {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Latest price
    #[serde(default)]
    pub c: Option<String>,
    /// Open price 24h
    #[serde(default)]
    pub o: Option<String>,
    /// High 24h
    #[serde(default)]
    pub h: Option<String>,
    /// Low 24h
    #[serde(default)]
    pub l: Option<String>,
    /// Volume 24h (base)
    #[serde(default)]
    pub a: Option<String>,
    /// Turnover 24h (quote)
    #[serde(default)]
    pub v: Option<String>,
    /// Change percentage
    #[serde(default)]
    pub r: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/q/deal
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtTradeRaw {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Price
    #[serde(default)]
    pub p: Option<String>,
    /// Volume/quantity
    #[serde(default)]
    pub a: Option<String>,
    /// Side: BUY or SELL
    #[serde(default)]
    pub m: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtTradeRaw {
    pub fn into_trade(self, fallback_symbol: &Symbol) -> Option<Trade> {
        let side = match self.m.as_deref() {
            Some("SELL") => Side::Sell,
            _ => Side::Buy,
        };
        Some(Trade {
            exchange: ExchangeId::XtFutures,
            symbol: self
                .s
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(xt_symbol_to_unified)
                .unwrap_or_else(|| fallback_symbol.clone()),
            price: Decimal::from_str(self.p.as_deref()?).ok()?,
            qty: Decimal::from_str(self.a.as_deref()?).ok()?,
            side,
            timestamp_ms: self.t.unwrap_or_else(now_ms),
            trade_id: None,
        })
    }
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/q/kline
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtKlineRaw {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Open
    #[serde(default)]
    pub o: Option<String>,
    /// Close
    #[serde(default)]
    pub c: Option<String>,
    /// High
    #[serde(default)]
    pub h: Option<String>,
    /// Low
    #[serde(default)]
    pub l: Option<String>,
    /// Volume (base)
    #[serde(default)]
    pub a: Option<String>,
    /// Turnover (quote)
    #[serde(default)]
    pub v: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtKlineRaw {
    pub fn into_candle(self, symbol: &Symbol, interval: Interval) -> Option<Candle> {
        let open = Decimal::from_str(self.o.as_deref()?).ok()?;
        let high = Decimal::from_str(self.h.as_deref()?).ok()?;
        let low = Decimal::from_str(self.l.as_deref()?).ok()?;
        let close = Decimal::from_str(self.c.as_deref()?).ok()?;
        let volume = self
            .a
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let open_time_ms = self.t?;
        let close_time_ms = open_time_ms + interval.as_secs() * 1000;

        Some(Candle {
            exchange: ExchangeId::XtFutures,
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
// REST: /future/market/v1/public/q/funding-rate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtFundingRateRaw {
    /// Symbol
    #[serde(default)]
    pub symbol: Option<String>,
    /// Funding rate (may come as number or string)
    #[serde(rename = "fundingRate", default)]
    pub funding_rate: Option<serde_json::Value>,
    /// Next collection/funding time (ms)
    #[serde(rename = "nextCollectionTime", default)]
    pub next_collection_time: Option<u64>,
    /// Collection interval (hours)
    #[serde(rename = "collectionInternal", default)]
    pub collection_internal: Option<u32>,
}

impl XtFundingRateRaw {
    pub fn into_funding_rate(self, fallback_symbol: &Symbol) -> FundingRate {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(xt_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        let rate = self
            .funding_rate
            .as_ref()
            .and_then(|v| parse_decimal_value(v))
            .unwrap_or_default();

        FundingRate {
            exchange: ExchangeId::XtFutures,
            symbol,
            rate,
            next_funding_time_ms: self.next_collection_time.unwrap_or(0),
            timestamp_ms: now_ms(),
        }
    }
}

/// Parse a serde_json::Value to Decimal (handles both string and number).
fn parse_decimal_value(v: &serde_json::Value) -> Option<Decimal> {
    if let Some(s) = v.as_str() {
        Decimal::from_str(s).ok()
    } else if let Some(n) = v.as_f64() {
        Decimal::try_from(n).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/q/symbol-mark-price
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtMarkPriceRaw {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Mark price
    #[serde(default)]
    pub p: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

// ---------------------------------------------------------------------------
// REST: /future/market/v1/public/contract/open-interest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtOpenInterestRaw {
    /// Symbol
    #[serde(default)]
    pub symbol: Option<String>,
    /// Open interest (contracts/coins)
    #[serde(rename = "openInterest", default)]
    pub open_interest: Option<String>,
    /// Open interest value in USD
    #[serde(rename = "openInterestUsd", default)]
    pub open_interest_usd: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub time: Option<u64>,
}

impl XtOpenInterestRaw {
    pub fn into_open_interest(self, fallback_symbol: &Symbol) -> OpenInterest {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(xt_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        OpenInterest {
            exchange: ExchangeId::XtFutures,
            symbol,
            open_interest: self
                .open_interest
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            open_interest_value: self
                .open_interest_usd
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            timestamp_ms: self.time.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// WS: depth channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtWsDepthData {
    /// Asks: [[price, qty], ...]
    #[serde(default)]
    pub a: Vec<Vec<String>>,
    /// Bids: [[price, qty], ...]
    #[serde(default)]
    pub b: Vec<Vec<String>>,
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtWsDepthData {
    pub fn into_orderbook(self, symbol: &Symbol, ts: u64) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::XtFutures,
            symbol: symbol.clone(),
            bids: parse_levels_vec(&self.b),
            asks: parse_levels_vec(&self.a),
            timestamp_ms: self.t.unwrap_or(ts),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: trade channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtWsTradeData {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Price
    #[serde(default)]
    pub p: Option<String>,
    /// Amount/quantity
    #[serde(default)]
    pub a: Option<String>,
    /// Side: BID (buy) or ASK (sell)
    #[serde(default)]
    pub m: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtWsTradeData {
    pub fn into_trade(self, symbol: &Symbol) -> Option<Trade> {
        let side = match self.m.as_deref() {
            Some("ASK") => Side::Sell,
            _ => Side::Buy,
        };
        Some(Trade {
            exchange: ExchangeId::XtFutures,
            symbol: symbol.clone(),
            price: Decimal::from_str(self.p.as_deref()?).ok()?,
            qty: Decimal::from_str(self.a.as_deref()?).ok()?,
            side,
            timestamp_ms: self.t.unwrap_or_else(now_ms),
            trade_id: None,
        })
    }
}

// ---------------------------------------------------------------------------
// WS: kline channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtWsKlineData {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Open
    #[serde(default)]
    pub o: Option<String>,
    /// Close
    #[serde(default)]
    pub c: Option<String>,
    /// High
    #[serde(default)]
    pub h: Option<String>,
    /// Low
    #[serde(default)]
    pub l: Option<String>,
    /// Volume (base)
    #[serde(default)]
    pub a: Option<String>,
    /// Turnover (quote)
    #[serde(default)]
    pub v: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtWsKlineData {
    pub fn into_candle(self, symbol: &Symbol, interval: Interval) -> Option<Candle> {
        let open = Decimal::from_str(self.o.as_deref()?).ok()?;
        let high = Decimal::from_str(self.h.as_deref()?).ok()?;
        let low = Decimal::from_str(self.l.as_deref()?).ok()?;
        let close = Decimal::from_str(self.c.as_deref()?).ok()?;
        let volume = self
            .a
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let ts = self.t.unwrap_or_else(now_ms);
        let interval_ms = interval.as_secs() * 1000;
        let open_time_ms = ts - (ts % interval_ms);
        let close_time_ms = open_time_ms + interval_ms;

        Some(Candle {
            exchange: ExchangeId::XtFutures,
            symbol: symbol.clone(),
            open,
            high,
            low,
            close,
            volume,
            open_time_ms,
            close_time_ms,
            is_closed: false,
        })
    }
}

// ---------------------------------------------------------------------------
// WS: mark_price channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtWsMarkPriceData {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Mark price
    #[serde(default)]
    pub p: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtWsMarkPriceData {
    pub fn into_mark_price(self, symbol: &Symbol, index_price: Decimal) -> MarkPrice {
        let mp = self
            .p
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        MarkPrice {
            exchange: ExchangeId::XtFutures,
            symbol: symbol.clone(),
            mark_price: mp,
            index_price,
            timestamp_ms: self.t.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// WS: agg_ticker channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct XtWsAggTickerData {
    /// Symbol
    #[serde(default)]
    pub s: Option<String>,
    /// Open price
    #[serde(default)]
    pub o: Option<String>,
    /// Close/latest price
    #[serde(default)]
    pub c: Option<String>,
    /// High
    #[serde(default)]
    pub h: Option<String>,
    /// Low
    #[serde(default)]
    pub l: Option<String>,
    /// Volume (base)
    #[serde(default)]
    pub a: Option<String>,
    /// Turnover (quote)
    #[serde(default)]
    pub v: Option<String>,
    /// Change ratio
    #[serde(default)]
    pub ch: Option<String>,
    /// Index price
    #[serde(default)]
    pub i: Option<String>,
    /// Mark price
    #[serde(default)]
    pub m: Option<String>,
    /// Best bid price
    #[serde(default)]
    pub bp: Option<String>,
    /// Best ask price
    #[serde(default)]
    pub ap: Option<String>,
    /// Timestamp
    #[serde(default)]
    pub t: Option<u64>,
}

impl XtWsAggTickerData {
    pub fn into_mark_price(self, symbol: &Symbol) -> MarkPrice {
        let mp = self
            .m
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let ip = self
            .i
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(mp);

        MarkPrice {
            exchange: ExchangeId::XtFutures,
            symbol: symbol.clone(),
            mark_price: mp,
            index_price: ip,
            timestamp_ms: self.t.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse price/qty pairs from XT format `[["price","qty"], ...]`.
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
    fn test_unified_to_xt() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_xt(&sym), "btc_usdt");
    }

    #[test]
    fn test_xt_symbol_to_unified() {
        let sym = xt_symbol_to_unified("btc_usdt");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = xt_symbol_to_unified("eth_usdt");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDT");

        let sym3 = xt_symbol_to_unified("sol_usdc");
        assert_eq!(sym3.base, "SOL");
        assert_eq!(sym3.quote, "USDC");
    }

    #[test]
    fn test_interval_to_xt() {
        assert_eq!(interval_to_xt(Interval::M1), "1m");
        assert_eq!(interval_to_xt(Interval::M5), "5m");
        assert_eq!(interval_to_xt(Interval::H1), "1h");
        assert_eq!(interval_to_xt(Interval::H4), "4h");
        assert_eq!(interval_to_xt(Interval::D1), "1d");
        assert_eq!(interval_to_xt(Interval::W1), "1w");
    }

    #[test]
    fn test_depth_into_orderbook() {
        let raw: XtDepthRaw = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "a": [["50001.5", "0.5"], ["50002.0", "1.0"]],
                "b": [["50000.0", "2.0"], ["49999.5", "1.5"]],
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(&sym);
        assert_eq!(ob.exchange, ExchangeId::XtFutures);
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 2);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.bids[0].qty, dec!(2.0));
        assert_eq!(ob.asks[0].price, dec!(50001.5));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_agg_ticker_into_ticker() {
        let raw: XtAggTickerRaw = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "o": "49000.0",
                "c": "50000.0",
                "h": "51000.0",
                "l": "48000.0",
                "a": "12345.678",
                "v": "617283900.0",
                "r": "0.0204",
                "i": "50100.0",
                "m": "50050.0",
                "bp": "49999.0",
                "ap": "50001.0",
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker(None);
        assert_eq!(ticker.exchange, ExchangeId::XtFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000.0));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.bid, Some(dec!(49999.0)));
        assert_eq!(ticker.ask, Some(dec!(50001.0)));
        assert!(ticker.price_change_pct_24h.is_some());
    }

    #[test]
    fn test_agg_ticker_into_mark_price() {
        let raw: XtAggTickerRaw = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "m": "50050.0",
                "i": "50100.0",
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let mp = raw.into_mark_price(&sym);
        assert_eq!(mp.mark_price, dec!(50050.0));
        assert_eq!(mp.index_price, dec!(50100.0));
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_into_trade() {
        let raw: XtTradeRaw = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "p": "50000.5",
                "a": "0.1",
                "m": "BUY",
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.exchange, ExchangeId::XtFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
        assert_eq!(trade.qty, dec!(0.1));
    }

    #[test]
    fn test_trade_sell_side() {
        let raw: XtTradeRaw = serde_json::from_str(
            r#"{"p": "50000.5", "a": "0.5", "m": "SELL", "t": 1700000000000}"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_kline_into_candle() {
        let raw: XtKlineRaw = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "o": "60000",
                "h": "60001",
                "c": "60000",
                "l": "59989.2",
                "a": "100.5",
                "v": "6030050.25",
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let candle = raw.into_candle(&sym, Interval::M1).unwrap();
        assert_eq!(candle.exchange, ExchangeId::XtFutures);
        assert_eq!(candle.open, dec!(60000));
        assert_eq!(candle.high, dec!(60001));
        assert_eq!(candle.close, dec!(60000));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000);
    }

    #[test]
    fn test_funding_rate_into_funding_rate() {
        let raw: XtFundingRateRaw = serde_json::from_str(
            r#"{
                "symbol": "btc_usdt",
                "fundingRate": -3.2516472e-05,
                "nextCollectionTime": 1772726400000,
                "collectionInternal": 8
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let fr = raw.into_funding_rate(&sym);
        assert_eq!(fr.exchange, ExchangeId::XtFutures);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDT"));
        assert!(fr.rate != Decimal::ZERO);
        assert_eq!(fr.next_funding_time_ms, 1772726400000);
    }

    #[test]
    fn test_funding_rate_string_value() {
        let raw: XtFundingRateRaw = serde_json::from_str(
            r#"{
                "symbol": "eth_usdt",
                "fundingRate": "0.0005",
                "nextCollectionTime": 1770710400000,
                "collectionInternal": 8
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("ETH", "USDT");
        let fr = raw.into_funding_rate(&sym);
        assert_eq!(fr.rate, dec!(0.0005));
    }

    #[test]
    fn test_ws_depth_into_orderbook() {
        let raw: XtWsDepthData = serde_json::from_str(
            r#"{
                "b": [["50000.0", "1.5"]],
                "a": [["50001.0", "2.0"]],
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(&sym, 1700000000000);
        assert_eq!(ob.exchange, ExchangeId::XtFutures);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.asks[0].price, dec!(50001.0));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_trade_into_trade() {
        let raw: XtWsTradeData = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "p": "50000.5",
                "a": "0.1",
                "m": "BID",
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.exchange, ExchangeId::XtFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
    }

    #[test]
    fn test_ws_trade_sell_side() {
        let raw: XtWsTradeData = serde_json::from_str(
            r#"{"p": "50000.5", "a": "0.5", "m": "ASK", "t": 1700000000000}"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ws_kline_into_candle() {
        let raw: XtWsKlineData = serde_json::from_str(
            r#"{
                "s": "btc_usdt",
                "o": "50000.0",
                "h": "51000.0",
                "l": "49000.0",
                "c": "50500.0",
                "a": "100.5",
                "v": "5050000.0",
                "t": 1700000060000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let candle = raw.into_candle(&sym, Interval::M1).unwrap();
        assert_eq!(candle.exchange, ExchangeId::XtFutures);
        assert_eq!(candle.open, dec!(50000.0));
        assert_eq!(candle.close, dec!(50500.0));
        assert_eq!(candle.volume, dec!(100.5));
    }

    #[test]
    fn test_open_interest_into_open_interest() {
        let raw: XtOpenInterestRaw = serde_json::from_str(
            r#"{
                "symbol": "btc_usdt",
                "openInterest": "16033.7485",
                "openInterestUsd": "879433129.63492",
                "time": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let oi = raw.into_open_interest(&sym);
        assert_eq!(oi.exchange, ExchangeId::XtFutures);
        assert_eq!(oi.open_interest, dec!(16033.7485));
        assert_eq!(oi.open_interest_value, dec!(879433129.63492));
        assert_eq!(oi.timestamp_ms, 1700000000000);
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
    fn test_symbol_info_from_raw() {
        let raw: XtSymbolRaw = serde_json::from_str(
            r#"{
                "symbol": "btc_usdt",
                "baseCoin": "btc",
                "quoteCoin": "usdt",
                "baseCoinDisplayPrecision": 4,
                "pricePrecision": 1,
                "quantityPrecision": 0,
                "minQty": "1",
                "minNotional": "10",
                "minStepPrice": "0.1",
                "contractType": "PERPETUAL",
                "state": 0,
                "tradeSwitch": true
            }"#,
        )
        .unwrap();

        let info = raw.into_symbol_info();
        assert_eq!(info.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.raw_symbol, "btc_usdt");
        assert_eq!(info.base_precision, 4);
        assert_eq!(info.quote_precision, 1);
        assert_eq!(info.status, SymbolStatus::Trading);
        assert_eq!(info.min_qty, Some(dec!(1)));
        assert_eq!(info.min_notional, Some(dec!(10)));
        assert_eq!(info.tick_size, Some(dec!(0.1)));
    }
}
