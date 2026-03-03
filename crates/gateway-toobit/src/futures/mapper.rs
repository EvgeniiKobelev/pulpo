use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to a Toobit futures symbol
/// (e.g. `Symbol::new("BTC","USDT")` -> `"BTC-SWAP-USDT"`).
pub fn unified_to_toobit(symbol: &Symbol) -> String {
    format!("{}-SWAP-{}", symbol.base, symbol.quote)
}

/// Convert a Toobit futures symbol to a unified Symbol
/// (e.g. `"BTC-SWAP-USDT"` -> `Symbol::new("BTC","USDT")`).
pub fn toobit_symbol_to_unified(raw: &str) -> Symbol {
    // Expected format: "BTC-SWAP-USDT"
    let parts: Vec<&str> = raw.split('-').collect();
    if parts.len() == 3 && parts[1] == "SWAP" {
        Symbol::new(parts[0], parts[2])
    } else if let Some((base, quote)) = raw.split_once('-') {
        Symbol::new(base, quote)
    } else {
        // Fallback for concatenated format like "BTCUSDT"
        let mid = raw.len().saturating_sub(4);
        Symbol::new(&raw[..mid], &raw[mid..])
    }
}

/// Map a unified Interval to Toobit kline interval string.
///
/// Toobit uses: 1m, 3m, 5m, 15m, 30m, 1h, 2h, 4h, 6h, 8h, 12h, 1d, 1w, 1M
pub fn interval_to_toobit(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1m", // Toobit doesn't support 1s, fallback to 1m
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

/// Map a unified Interval to Toobit WS kline topic suffix.
///
/// Toobit WS kline topics: kline_1m, kline_5m, kline_15m, kline_30m,
/// kline_1h, kline_2h, kline_4h, kline_6h, kline_12h, kline_1d, kline_1w, kline_1M
pub fn interval_to_toobit_ws(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 | Interval::M1 => "kline_1m",
        Interval::M3 => "kline_3m",
        Interval::M5 => "kline_5m",
        Interval::M15 => "kline_15m",
        Interval::M30 => "kline_30m",
        Interval::H1 => "kline_1h",
        Interval::H4 => "kline_4h",
        Interval::D1 => "kline_1d",
        Interval::W1 => "kline_1w",
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/exchangeInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ToobitExchangeInfoResponse {
    #[serde(default)]
    pub symbols: Vec<ToobitSymbolInfo>,
    #[serde(default)]
    pub contracts: Vec<ToobitContractInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ToobitSymbolInfo {
    #[serde(default)]
    pub symbol: String,
    #[serde(rename = "symbolName", default)]
    pub symbol_name: Option<String>,
    #[serde(rename = "baseAsset", default)]
    pub base_asset: String,
    #[serde(rename = "quoteAsset", default)]
    pub quote_asset: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub filters: Vec<ToobitFilter>,
}

#[derive(Debug, Deserialize)]
pub struct ToobitFilter {
    #[serde(rename = "filterType", default)]
    pub filter_type: String,
    #[serde(rename = "minPrice", default)]
    pub min_price: Option<String>,
    #[serde(rename = "maxPrice", default)]
    pub max_price: Option<String>,
    #[serde(rename = "tickSize", default)]
    pub tick_size: Option<String>,
    #[serde(rename = "minQty", default)]
    pub min_qty: Option<String>,
    #[serde(rename = "maxQty", default)]
    pub max_qty: Option<String>,
    #[serde(rename = "stepSize", default)]
    pub step_size: Option<String>,
    #[serde(rename = "minNotional", default)]
    pub min_notional: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ToobitContractInfo {
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(rename = "baseAsset", default)]
    pub base_asset: String,
    #[serde(rename = "quoteAsset", default)]
    pub quote_asset: String,
    #[serde(default)]
    pub filters: Vec<ToobitFilter>,
}

impl ToobitExchangeInfoResponse {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .contracts
            .into_iter()
            .filter(|c| c.status.as_deref() != Some("BREAK"))
            .map(|c| {
                let symbol = toobit_symbol_to_unified(&c.symbol);

                let mut tick_size: Option<Decimal> = None;
                let mut min_qty: Option<Decimal> = None;
                let mut min_notional: Option<Decimal> = None;
                let mut base_precision: u8 = 2;
                let mut quote_precision: u8 = 2;

                for f in &c.filters {
                    match f.filter_type.as_str() {
                        "PRICE_FILTER" => {
                            tick_size =
                                f.tick_size.as_deref().and_then(|s| Decimal::from_str(s).ok());
                            if let Some(ts) = tick_size {
                                quote_precision = ts.scale() as u8;
                            }
                        }
                        "LOT_SIZE" => {
                            min_qty =
                                f.min_qty.as_deref().and_then(|s| Decimal::from_str(s).ok());
                            if let Some(ss) =
                                f.step_size.as_deref().and_then(|s| Decimal::from_str(s).ok())
                            {
                                base_precision = ss.scale() as u8;
                            }
                        }
                        "MIN_NOTIONAL" => {
                            min_notional = f
                                .min_notional
                                .as_deref()
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        _ => {}
                    }
                }

                SymbolInfo {
                    raw_symbol: c.symbol,
                    symbol,
                    status: SymbolStatus::Trading,
                    base_precision,
                    quote_precision,
                    min_qty,
                    min_notional,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::ToobitFutures,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /quote/v1/depth
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ToobitOrderBookRaw {
    #[serde(default)]
    pub b: Vec<[String; 2]>,
    #[serde(default)]
    pub a: Vec<[String; 2]>,
    #[serde(default)]
    pub t: Option<u64>,
}

impl ToobitOrderBookRaw {
    pub fn into_orderbook(self, symbol: &Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::ToobitFutures,
            symbol: symbol.clone(),
            bids: parse_levels(&self.b),
            asks: parse_levels(&self.a),
            timestamp_ms: self.t.unwrap_or(0),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /quote/v1/trades
// ---------------------------------------------------------------------------

/// Toobit REST trades response.
///
/// Real API format:
/// ```json
/// {"t":1772556711718,"p":"68019.4","q":"116","ibm":true}
/// ```
#[derive(Debug, Deserialize)]
pub struct ToobitTradeRaw {
    /// Timestamp (ms)
    #[serde(default)]
    pub t: Option<u64>,
    /// Price
    #[serde(default)]
    pub p: Option<String>,
    /// Quantity
    #[serde(default)]
    pub q: Option<String>,
    /// Is buyer maker
    #[serde(default)]
    pub ibm: Option<bool>,
    // Legacy field names (documentation vs reality)
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub time: Option<u64>,
    #[serde(rename = "isBuyerMaker", default)]
    pub is_buyer_maker: Option<bool>,
}

impl ToobitTradeRaw {
    pub fn into_trade(self, symbol: &Symbol) -> Option<Trade> {
        let price_str = self.p.or(self.price)?;
        let qty_str = self.q.or(self.quantity)?;
        let is_maker = self.ibm.or(self.is_buyer_maker).unwrap_or(false);
        let ts = self.t.or(self.time).unwrap_or(0);

        let side = if is_maker { Side::Sell } else { Side::Buy };
        Some(Trade {
            exchange: ExchangeId::ToobitFutures,
            symbol: symbol.clone(),
            price: Decimal::from_str(&price_str).ok()?,
            qty: Decimal::from_str(&qty_str).ok()?,
            side,
            timestamp_ms: ts,
            trade_id: None,
        })
    }
}

// ---------------------------------------------------------------------------
// REST: /quote/v1/klines
// ---------------------------------------------------------------------------

/// Toobit kline row.
///
/// Real API format: `[openTime, open, high, low, close, volume, 0, quoteVolume, ...]`
/// Note: index 6 is always 0 in real API, not a close time.
pub fn parse_kline_row(
    row: &[serde_json::Value],
    symbol: &Symbol,
    interval: Interval,
) -> Option<Candle> {
    if row.len() < 6 {
        return None;
    }
    let open_time_ms = row[0].as_u64().or_else(|| row[0].as_str()?.parse().ok())?;
    let open = parse_decimal_value(&row[1])?;
    let high = parse_decimal_value(&row[2])?;
    let low = parse_decimal_value(&row[3])?;
    let close = parse_decimal_value(&row[4])?;
    let volume = parse_decimal_value(&row[5]).unwrap_or_default();
    let close_time_ms = open_time_ms + interval.as_secs() * 1000;

    Some(Candle {
        exchange: ExchangeId::ToobitFutures,
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

// ---------------------------------------------------------------------------
// REST: /quote/v1/contract/ticker/24hr
// ---------------------------------------------------------------------------

/// Toobit 24hr ticker response.
///
/// Real API format:
/// ```json
/// {"t":1772556696120,"s":"BTC-SWAP-USDT","c":"68041.9","h":"69790.8",
///  "l":"66113.3","o":"69700","b":"68041.9","a":"68043.4","v":"61898029",
///  "qv":"4204345766.952","op":"221301.998","pc":"-1658.1","pcp":"-0.0238"}
/// ```
#[derive(Debug, Deserialize)]
pub struct ToobitTickerRaw {
    #[serde(default)]
    pub t: Option<u64>,
    /// Symbol (e.g. "BTC-SWAP-USDT")
    #[serde(default)]
    pub s: Option<String>,
    /// Close / last price
    #[serde(default)]
    pub c: Option<String>,
    /// 24h high
    #[serde(default)]
    pub h: Option<String>,
    /// 24h low
    #[serde(default)]
    pub l: Option<String>,
    /// 24h open
    #[serde(default)]
    pub o: Option<String>,
    /// Best bid price
    #[serde(default)]
    pub b: Option<String>,
    /// Best ask price
    #[serde(default)]
    pub a: Option<String>,
    /// 24h volume (base)
    #[serde(default)]
    pub v: Option<String>,
    /// 24h quote volume
    #[serde(default)]
    pub qv: Option<String>,
    /// Price change
    #[serde(default)]
    pub pc: Option<String>,
    /// Price change percent (e.g. "-0.0238")
    #[serde(default)]
    pub pcp: Option<String>,
}

impl ToobitTickerRaw {
    pub fn into_ticker(self, fallback_symbol: Option<&Symbol>) -> Ticker {
        let raw_sym = self.s.as_deref().unwrap_or("");
        let symbol = if !raw_sym.is_empty() {
            toobit_symbol_to_unified(raw_sym)
        } else if let Some(s) = fallback_symbol {
            s.clone()
        } else {
            Symbol::new("UNKNOWN", "UNKNOWN")
        };

        let last_price = self
            .c
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        let bid = self
            .b
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());

        let ask = self
            .a
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());

        let volume_24h = self
            .v
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        // pcp is a raw ratio like "-0.0238", convert to percentage
        let pct = self
            .pcp
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .map(|d| d * Decimal::from(100));

        Ticker {
            exchange: ExchangeId::ToobitFutures,
            symbol,
            last_price,
            bid,
            ask,
            volume_24h,
            price_change_pct_24h: pct,
            timestamp_ms: self.t.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/futures/fundingRate
// ---------------------------------------------------------------------------

/// Toobit funding rate response.
///
/// Real API format:
/// ```json
/// {"symbol":"BTC-SWAP-USDT","rate":"0.00002195","nextFundingTime":"1772582400000"}
/// ```
#[derive(Debug, Deserialize)]
pub struct ToobitFundingRateRaw {
    pub symbol: String,
    pub rate: Option<String>,
    #[serde(rename = "nextFundingTime", default)]
    pub next_funding_time: Option<String>,
}

impl ToobitFundingRateRaw {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = toobit_symbol_to_unified(&self.symbol);
        FundingRate {
            exchange: ExchangeId::ToobitFutures,
            symbol,
            rate: self
                .rate
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
// REST: /quote/v1/markPrice
// ---------------------------------------------------------------------------

/// Toobit mark price response.
///
/// Real API format:
/// ```json
/// {"exchangeId":301,"symbolId":"BTC-SWAP-USDT","price":"68022","time":1772556703000}
/// ```
#[derive(Debug, Deserialize)]
pub struct ToobitMarkPriceRaw {
    #[serde(rename = "symbolId", default)]
    pub symbol_id: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    /// Mark price (the single "price" field in real API)
    #[serde(default)]
    pub price: Option<String>,
    #[serde(rename = "markPrice", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "indexPrice", default)]
    pub index_price: Option<String>,
    #[serde(default)]
    pub time: Option<u64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(rename = "exchangeId", default)]
    pub exchange_id: Option<u64>,
}

impl ToobitMarkPriceRaw {
    pub fn into_mark_price(self, fallback_symbol: &Symbol) -> MarkPrice {
        let symbol = self
            .symbol_id
            .as_deref()
            .or(self.symbol.as_deref())
            .map(toobit_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        let mp = self
            .price
            .as_deref()
            .or(self.mark_price.as_deref())
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();

        MarkPrice {
            exchange: ExchangeId::ToobitFutures,
            symbol,
            mark_price: mp,
            index_price: self
                .index_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(mp), // fallback to mark price if no index price
            timestamp_ms: self.time.or(self.timestamp).unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// WS: trade stream
// ---------------------------------------------------------------------------

/// WebSocket trade data.
/// Fields: "v" = trade ID, "t" = timestamp, "p" = price, "q" = quantity, "m" = isBuyerMaker
#[derive(Debug, Deserialize)]
pub struct ToobitWsTradeData {
    #[serde(default)]
    pub v: Option<String>,
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub p: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub m: Option<bool>,
}

impl ToobitWsTradeData {
    pub fn into_trade(self, symbol: &Symbol) -> Option<Trade> {
        let side = if self.m.unwrap_or(false) {
            Side::Sell
        } else {
            Side::Buy
        };
        Some(Trade {
            exchange: ExchangeId::ToobitFutures,
            symbol: symbol.clone(),
            price: Decimal::from_str(self.p.as_deref()?).ok()?,
            qty: Decimal::from_str(self.q.as_deref()?).ok()?,
            side,
            timestamp_ms: self.t.unwrap_or(0),
            trade_id: self.v,
        })
    }
}

// ---------------------------------------------------------------------------
// WS: depth stream
// ---------------------------------------------------------------------------

/// WebSocket orderbook data.
#[derive(Debug, Deserialize)]
pub struct ToobitWsDepthInner {
    #[serde(default)]
    pub s: Option<String>,
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub v: Option<String>,
    #[serde(default)]
    pub b: Vec<[String; 2]>,
    #[serde(default)]
    pub a: Vec<[String; 2]>,
}

impl ToobitWsDepthInner {
    pub fn into_orderbook(self, symbol: &Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::ToobitFutures,
            symbol: symbol.clone(),
            bids: parse_levels(&self.b),
            asks: parse_levels(&self.a),
            timestamp_ms: self.t.unwrap_or(0),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: kline stream
// ---------------------------------------------------------------------------

/// WebSocket kline data.
#[derive(Debug, Deserialize)]
pub struct ToobitWsKlineData {
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub s: Option<String>,
    #[serde(default)]
    pub sn: Option<String>,
    #[serde(default)]
    pub o: Option<String>,
    #[serde(default)]
    pub h: Option<String>,
    #[serde(default)]
    pub l: Option<String>,
    #[serde(default)]
    pub c: Option<String>,
    #[serde(default)]
    pub v: Option<String>,
    #[serde(default)]
    pub e: Option<u64>,
}

impl ToobitWsKlineData {
    pub fn into_candle(self, symbol: &Symbol, interval: Interval) -> Option<Candle> {
        let open_time_ms = self.t?;
        let interval_ms = interval.as_secs() * 1000;
        let close_time_ms = self.e.unwrap_or(open_time_ms + interval_ms);

        Some(Candle {
            exchange: ExchangeId::ToobitFutures,
            symbol: symbol.clone(),
            open: Decimal::from_str(self.o.as_deref()?).ok()?,
            high: Decimal::from_str(self.h.as_deref()?).ok()?,
            low: Decimal::from_str(self.l.as_deref()?).ok()?,
            close: Decimal::from_str(self.c.as_deref()?).ok()?,
            volume: self
                .v
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
// WS: realtimes (24hr ticker) stream
// ---------------------------------------------------------------------------

/// WebSocket 24hr ticker data.
#[derive(Debug, Deserialize)]
pub struct ToobitWsTickerData {
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub s: Option<String>,
    #[serde(default)]
    pub c: Option<String>,
    #[serde(default)]
    pub h: Option<String>,
    #[serde(default)]
    pub l: Option<String>,
    #[serde(default)]
    pub o: Option<String>,
    #[serde(default)]
    pub v: Option<String>,
    #[serde(default)]
    pub qv: Option<String>,
}

// ---------------------------------------------------------------------------
// WS: markPrice stream
// ---------------------------------------------------------------------------

/// WebSocket mark price data.
#[derive(Debug, Deserialize)]
pub struct ToobitWsMarkPriceData {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(rename = "markPrice", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "indexPrice", default)]
    pub index_price: Option<String>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl ToobitWsMarkPriceData {
    pub fn into_mark_price(self, fallback_symbol: &Symbol) -> MarkPrice {
        let symbol = self
            .symbol
            .as_deref()
            .map(toobit_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        MarkPrice {
            exchange: ExchangeId::ToobitFutures,
            symbol,
            mark_price: self
                .mark_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            index_price: self
                .index_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse price/size pairs from Toobit orderbook format `[[price, qty], ...]`.
fn parse_levels(raw: &[[String; 2]]) -> Vec<Level> {
    raw.iter()
        .filter_map(|pair| {
            let price = Decimal::from_str(&pair[0]).ok()?;
            let qty = Decimal::from_str(&pair[1]).ok()?;
            Some(Level::new(price, qty))
        })
        .collect()
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

/// Current time in milliseconds.
fn now_ms() -> u64 {
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
    fn test_unified_to_toobit() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_toobit(&sym), "BTC-SWAP-USDT");
    }

    #[test]
    fn test_toobit_symbol_to_unified() {
        let sym = toobit_symbol_to_unified("BTC-SWAP-USDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = toobit_symbol_to_unified("ETH-SWAP-USDT");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDT");
    }

    #[test]
    fn test_interval_to_toobit() {
        assert_eq!(interval_to_toobit(Interval::M1), "1m");
        assert_eq!(interval_to_toobit(Interval::M5), "5m");
        assert_eq!(interval_to_toobit(Interval::H1), "1h");
        assert_eq!(interval_to_toobit(Interval::H4), "4h");
        assert_eq!(interval_to_toobit(Interval::D1), "1d");
        assert_eq!(interval_to_toobit(Interval::W1), "1w");
    }

    #[test]
    fn test_orderbook_into_orderbook() {
        let raw: ToobitOrderBookRaw = serde_json::from_str(
            r#"{
                "b": [["50000.0", "2.0"], ["49999.5", "1.5"]],
                "a": [["50001.5", "0.5"], ["50002.0", "1.0"]],
                "t": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(&sym);
        assert_eq!(ob.exchange, ExchangeId::ToobitFutures);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 2);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.bids[0].qty, dec!(2.0));
        assert_eq!(ob.asks[0].price, dec!(50001.5));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_into_trade() {
        let raw: ToobitTradeRaw = serde_json::from_str(
            r#"{
                "t": 1700000000000,
                "p": "50000.5",
                "q": "0.1",
                "ibm": false
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.exchange, ExchangeId::ToobitFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
        assert_eq!(trade.qty, dec!(0.1));
    }

    #[test]
    fn test_trade_buyer_maker() {
        let raw: ToobitTradeRaw = serde_json::from_str(
            r#"{
                "t": 1700000000000,
                "p": "50000.5",
                "q": "0.5",
                "ibm": true
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ticker_into_ticker() {
        let raw: ToobitTickerRaw = serde_json::from_str(
            r#"{
                "t": 1700000000000,
                "s": "BTC-SWAP-USDT",
                "c": "50000.0",
                "h": "51000.0",
                "l": "48000.0",
                "o": "49000.0",
                "b": "49999.5",
                "a": "50001.5",
                "v": "12345.678",
                "qv": "617283900.0",
                "pcp": "0.0204"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker(None);
        assert_eq!(ticker.exchange, ExchangeId::ToobitFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000.0));
        assert_eq!(ticker.bid, Some(dec!(49999.5)));
        assert_eq!(ticker.ask, Some(dec!(50001.5)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(2.04)));
        assert_eq!(ticker.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_funding_rate_into_funding_rate() {
        let raw: ToobitFundingRateRaw = serde_json::from_str(
            r#"{
                "symbol": "BTC-SWAP-USDT",
                "rate": "0.000123",
                "nextFundingTime": "1700028800000"
            }"#,
        )
        .unwrap();

        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::ToobitFutures);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(fr.rate, dec!(0.000123));
        assert_eq!(fr.next_funding_time_ms, 1700028800000);
    }

    #[test]
    fn test_mark_price_into_mark_price() {
        let raw: ToobitMarkPriceRaw = serde_json::from_str(
            r#"{
                "exchangeId": 301,
                "symbolId": "BTC-SWAP-USDT",
                "price": "50123.5",
                "time": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let mp = raw.into_mark_price(&sym);
        assert_eq!(mp.mark_price, dec!(50123.5));
        assert_eq!(mp.index_price, dec!(50123.5)); // falls back to mark price
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_trade_into_trade() {
        let raw: ToobitWsTradeData = serde_json::from_str(
            r#"{
                "v": "123456",
                "t": 1700000000000,
                "p": "50000.5",
                "q": "0.1",
                "m": false
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let trade = raw.into_trade(&sym).unwrap();
        assert_eq!(trade.exchange, ExchangeId::ToobitFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
        assert_eq!(trade.qty, dec!(0.1));
        assert_eq!(trade.trade_id, Some("123456".to_string()));
    }

    #[test]
    fn test_ws_depth_into_orderbook() {
        let raw: ToobitWsDepthInner = serde_json::from_str(
            r#"{
                "s": "BTC-SWAP-USDT",
                "t": 1700000000000,
                "v": "100",
                "b": [["50000.0", "1.5"]],
                "a": [["50001.0", "2.0"]]
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let ob = raw.into_orderbook(&sym);
        assert_eq!(ob.exchange, ExchangeId::ToobitFutures);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.asks[0].price, dec!(50001.0));
    }

    #[test]
    fn test_ws_kline_into_candle() {
        let raw: ToobitWsKlineData = serde_json::from_str(
            r#"{
                "t": 1700000000000,
                "s": "BTC-SWAP-USDT",
                "o": "50000.0",
                "h": "51000.0",
                "l": "49000.0",
                "c": "50500.0",
                "v": "100.5",
                "e": 1700000060000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let candle = raw.into_candle(&sym, Interval::M1).unwrap();
        assert_eq!(candle.exchange, ExchangeId::ToobitFutures);
        assert_eq!(candle.open, dec!(50000.0));
        assert_eq!(candle.high, dec!(51000.0));
        assert_eq!(candle.low, dec!(49000.0));
        assert_eq!(candle.close, dec!(50500.0));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000);
    }

    #[test]
    fn test_parse_levels() {
        let raw = [
            ["100.50".to_string(), "1.5".to_string()],
            ["99.00".to_string(), "2.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(100.50));
        assert_eq!(levels[0].qty, dec!(1.5));
    }

    #[test]
    fn test_parse_kline_row() {
        let row = vec![
            serde_json::json!(1700000000000u64),
            serde_json::json!("50000.0"),
            serde_json::json!("51000.0"),
            serde_json::json!("49000.0"),
            serde_json::json!("50500.0"),
            serde_json::json!("100.5"),
            serde_json::json!(0),
            serde_json::json!("5050000.0"),
        ];
        let sym = Symbol::new("BTC", "USDT");
        let candle = parse_kline_row(&row, &sym, Interval::M1).unwrap();
        assert_eq!(candle.open, dec!(50000.0));
        assert_eq!(candle.close, dec!(50500.0));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000); // open + 60s
    }

    #[test]
    fn test_ws_mark_price_into_mark_price() {
        let raw: ToobitWsMarkPriceData = serde_json::from_str(
            r#"{
                "symbol": "BTC-SWAP-USDT",
                "markPrice": "50010.0",
                "indexPrice": "50005.0",
                "timestamp": 1700000000000
            }"#,
        )
        .unwrap();

        let sym = Symbol::new("BTC", "USDT");
        let mp = raw.into_mark_price(&sym);
        assert_eq!(mp.mark_price, dec!(50010.0));
        assert_eq!(mp.index_price, dec!(50005.0));
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }
}
