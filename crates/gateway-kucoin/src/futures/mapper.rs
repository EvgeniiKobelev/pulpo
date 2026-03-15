use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// KuCoin Futures API response wrapper: { "code": "200000", "data": ... }
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct KucoinFuturesResponse<T> {
    pub code: String,
    pub data: T,
}

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to a KuCoin Futures symbol (e.g. "XBTUSDTM").
///
/// KuCoin Futures uses "XBT" for Bitcoin instead of "BTC".
pub fn unified_to_kucoin_futures(symbol: &Symbol) -> String {
    let base = if symbol.base == "BTC" { "XBT" } else { &symbol.base };
    format!("{}{}M", base, symbol.quote)
}

/// Convert a KuCoin Futures symbol (e.g. "XBTUSDTM") to a unified Symbol
/// using baseCurrency/quoteCurrency from contract data.
pub fn kucoin_futures_pair_to_unified(
    base_currency: &str,
    quote_currency: &str,
) -> Symbol {
    // Normalize XBT -> BTC
    let base = if base_currency.eq_ignore_ascii_case("XBT") {
        "BTC"
    } else {
        base_currency
    };
    Symbol::new(base, quote_currency)
}

/// Try to parse a KuCoin futures symbol string (e.g. "BTCUSDTM") into a unified Symbol.
pub fn kucoin_futures_symbol_to_unified(raw: &str) -> Symbol {
    let s = raw.trim_end_matches('M');
    for quote in &["USDT", "USDC", "USD"] {
        if s.ends_with(quote) && s.len() > quote.len() {
            let base = &s[..s.len() - quote.len()];
            let base = if base.eq_ignore_ascii_case("XBT") {
                "BTC"
            } else {
                base
            };
            return Symbol::new(base, *quote);
        }
    }
    Symbol::new(s, "UNKNOWN")
}

/// Map a unified Interval to KuCoin Futures kline granularity (minutes).
pub fn interval_to_granularity(interval: Interval) -> u32 {
    match interval {
        Interval::S1 | Interval::M1 => 1,
        Interval::M3 => 5, // 3m not supported, fallback to 5m
        Interval::M5 => 5,
        Interval::M15 => 15,
        Interval::M30 => 30,
        Interval::H1 => 60,
        Interval::H4 => 240,
        Interval::D1 => 1440,
        Interval::W1 => 10080,
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/contracts/active
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfContractRaw {
    pub symbol: String,
    #[serde(default)]
    pub base_currency: Option<String>,
    #[serde(default)]
    pub quote_currency: Option<String>,
    #[serde(default)]
    pub tick_size: Option<f64>,
    #[serde(default)]
    pub lot_size: Option<f64>,
    #[serde(default)]
    pub multiplier: Option<f64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mark_price: Option<f64>,
    #[serde(default)]
    pub index_price: Option<f64>,
    #[serde(default)]
    pub last_trade_price: Option<f64>,
    #[serde(default)]
    pub open_interest: Option<String>,
    #[serde(default)]
    pub turnover_of24h: Option<f64>,
    #[serde(default)]
    pub volume_of24h: Option<f64>,
    #[serde(default)]
    pub price_change_pct_of24h: Option<f64>,
    #[serde(rename = "maxPrice", default)]
    pub max_price: Option<f64>,
    #[serde(rename = "lowPrice", default)]
    pub low_price: Option<f64>,
}

impl KfContractRaw {
    pub fn into_symbol_info(self) -> SymbolInfo {
        let base = self.base_currency.as_deref().unwrap_or("UNKNOWN");
        let quote = self.quote_currency.as_deref().unwrap_or("UNKNOWN");
        let symbol = kucoin_futures_pair_to_unified(base, quote);

        let status = match self.status.as_deref() {
            Some("Open") => SymbolStatus::Trading,
            Some("PrepareSettled") | Some("BeingSettled") | Some("Paused") => SymbolStatus::Halted,
            _ => SymbolStatus::Unknown,
        };

        let tick_size = self.tick_size.and_then(|v| Decimal::try_from(v).ok());
        let quote_precision = tick_size
            .map(|d| d.scale() as u8)
            .unwrap_or(2);

        SymbolInfo {
            symbol,
            raw_symbol: self.symbol,
            status,
            base_precision: 4,
            quote_precision,
            min_qty: self.lot_size.and_then(|v| Decimal::try_from(v).ok()),
            min_notional: None,
            tick_size,
        }
    }

    pub fn into_ticker(self) -> Ticker {
        let base = self.base_currency.as_deref().unwrap_or("UNKNOWN");
        let quote = self.quote_currency.as_deref().unwrap_or("UNKNOWN");
        let symbol = kucoin_futures_pair_to_unified(base, quote);

        Ticker {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            last_price: self
                .last_trade_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            bid: None,
            ask: None,
            volume_24h: self
                .volume_of24h
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            price_change_pct_24h: self
                .price_change_pct_of24h
                .and_then(|v| Decimal::try_from(v).ok())
                .map(|d| d * Decimal::from(100)),
            timestamp_ms: now_ms(),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/ticker
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfTickerRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub sequence: Option<u64>,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub size: Option<serde_json::Value>,
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub best_bid_price: Option<String>,
    #[serde(default)]
    pub best_bid_size: Option<serde_json::Value>,
    #[serde(default)]
    pub best_ask_price: Option<String>,
    #[serde(default)]
    pub best_ask_size: Option<serde_json::Value>,
    #[serde(default)]
    pub ts: Option<u64>,
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/24hr-stats
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kf24hrStatsRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub volume: Option<serde_json::Value>,
    #[serde(default)]
    pub turnover: Option<serde_json::Value>,
    #[serde(default)]
    pub last_price: Option<serde_json::Value>,
    #[serde(default)]
    pub price_chg_pct: Option<serde_json::Value>,
    #[serde(default)]
    pub ts: Option<u64>,
}

impl Kf24hrStatsRaw {
    pub fn into_ticker(self, fallback_symbol: &Symbol) -> Ticker {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(kucoin_futures_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        let ts = self.ts.map(ns_to_ms).unwrap_or_else(now_ms);

        Ticker {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            last_price: parse_value_decimal(self.last_price.as_ref()),
            bid: None,
            ask: None,
            volume_24h: parse_value_decimal(self.volume.as_ref()),
            price_change_pct_24h: parse_value_decimal_opt(self.price_chg_pct.as_ref())
                .map(|d| d * Decimal::from(100)),
            timestamp_ms: ts,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/level2/depth{20,100}
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfOrderBookRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub sequence: Option<u64>,
    #[serde(default)]
    pub asks: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    pub bids: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    pub ts: Option<u64>,
}

impl KfOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            bids: parse_levels_mixed(&self.bids),
            asks: parse_levels_mixed(&self.asks),
            timestamp_ms: self.ts.map(ns_to_ms).unwrap_or_else(now_ms),
            sequence: self.sequence,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/trade/history
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfTradeRaw {
    #[serde(default)]
    pub sequence: Option<u64>,
    #[serde(default)]
    pub trade_id: Option<String>,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub size: Option<serde_json::Value>,
    #[serde(default)]
    pub ts: Option<u64>,
}

impl KfTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        Trade {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            price: self
                .price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            qty: parse_value_decimal(self.size.as_ref()),
            side: parse_side(self.side.as_deref()),
            timestamp_ms: self.ts.map(ns_to_ms).unwrap_or_else(now_ms),
            trade_id: self.trade_id,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/funding-rate/{symbol}/current
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfFundingRateRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub granularity: Option<u64>,
    #[serde(default)]
    pub time_point: Option<u64>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub predicted_value: Option<serde_json::Value>,
}

impl KfFundingRateRaw {
    pub fn into_funding_rate(self, symbol: &Symbol) -> FundingRate {
        let next_funding = self
            .time_point
            .unwrap_or(0)
            .saturating_add(self.granularity.unwrap_or(28800000));

        FundingRate {
            exchange: ExchangeId::KucoinFutures,
            symbol: symbol.clone(),
            rate: parse_value_decimal(self.value.as_ref()),
            next_funding_time_ms: next_funding,
            timestamp_ms: self.time_point.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/mark-price/{symbol}/current
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfMarkPriceRaw {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub time_point: Option<u64>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub index_price: Option<serde_json::Value>,
}

impl KfMarkPriceRaw {
    pub fn into_mark_price(self, symbol: &Symbol) -> MarkPrice {
        let mp = parse_value_decimal(self.value.as_ref());
        let ip = parse_value_decimal(self.index_price.as_ref());

        MarkPrice {
            exchange: ExchangeId::KucoinFutures,
            symbol: symbol.clone(),
            mark_price: mp,
            index_price: if ip.is_zero() { mp } else { ip },
            timestamp_ms: self.time_point.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

/// Bullet token response (POST /api/v1/bullet-public)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfBulletResponse {
    pub token: String,
    pub instance_servers: Vec<KfInstanceServer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfInstanceServer {
    pub endpoint: String,
    #[serde(default)]
    pub ping_interval: Option<u64>,
    #[serde(default)]
    pub ping_timeout: Option<u64>,
}

/// Generic WS message envelope
#[derive(Debug, Deserialize)]
pub struct KfWsMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// WS depth snapshot data (level2Depth5/level2Depth50)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfWsDepthData {
    #[serde(default)]
    pub asks: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    pub bids: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    pub ts: Option<u64>,
    #[serde(default)]
    pub sequence: Option<u64>,
}

impl KfWsDepthData {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            bids: parse_levels_mixed(&self.bids),
            asks: parse_levels_mixed(&self.asks),
            timestamp_ms: self.ts.map(ns_to_ms).unwrap_or_else(now_ms),
            sequence: self.sequence,
        }
    }
}

/// WS trade/execution data
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfWsTradeData {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub size: Option<serde_json::Value>,
    #[serde(default)]
    pub trade_id: Option<String>,
    #[serde(default)]
    pub ts: Option<u64>,
}

impl KfWsTradeData {
    pub fn into_trade(self) -> Trade {
        let symbol = self
            .symbol
            .as_deref()
            .map(kucoin_futures_symbol_to_unified)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));

        Trade {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            price: self
                .price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            qty: parse_value_decimal(self.size.as_ref()),
            side: parse_side(self.side.as_deref()),
            timestamp_ms: self.ts.map(ns_to_ms).unwrap_or_else(now_ms),
            trade_id: self.trade_id,
        }
    }
}

/// WS instrument data (mark price + funding)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KfWsInstrumentData {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub mark_price: Option<serde_json::Value>,
    #[serde(default)]
    pub index_price: Option<serde_json::Value>,
    #[serde(default)]
    pub funding_rate: Option<serde_json::Value>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl KfWsInstrumentData {
    pub fn into_mark_price(self, fallback_symbol: &Symbol) -> MarkPrice {
        let symbol = self
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(kucoin_futures_symbol_to_unified)
            .unwrap_or_else(|| fallback_symbol.clone());

        let mp = parse_value_decimal(self.mark_price.as_ref());
        let ip = parse_value_decimal(self.index_price.as_ref());

        MarkPrice {
            exchange: ExchangeId::KucoinFutures,
            symbol,
            mark_price: mp,
            index_price: if ip.is_zero() { mp } else { ip },
            timestamp_ms: self.timestamp.unwrap_or_else(now_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Parse a serde_json::Value into Decimal (handles both strings and numbers).
pub fn parse_value_decimal(v: Option<&serde_json::Value>) -> Decimal {
    match v {
        Some(serde_json::Value::String(s)) => Decimal::from_str(s).unwrap_or_default(),
        Some(serde_json::Value::Number(n)) => n
            .as_f64()
            .and_then(|f| Decimal::try_from(f).ok())
            .unwrap_or_default(),
        _ => Decimal::ZERO,
    }
}

fn parse_value_decimal_opt(v: Option<&serde_json::Value>) -> Option<Decimal> {
    match v {
        Some(serde_json::Value::String(s)) => Decimal::from_str(s).ok(),
        Some(serde_json::Value::Number(n)) => n.as_f64().and_then(|f| Decimal::try_from(f).ok()),
        _ => None,
    }
}

/// Parse price/qty pairs that may be numbers or strings: `[[price, qty], ...]`.
pub fn parse_levels_mixed(raw: &[Vec<serde_json::Value>]) -> Vec<Level> {
    raw.iter()
        .filter_map(|pair| {
            if pair.len() < 2 {
                return None;
            }
            let price = parse_value_decimal_opt(Some(&pair[0]))?;
            let qty = parse_value_decimal_opt(Some(&pair[1]))?;
            Some(Level::new(price, qty))
        })
        .collect()
}

fn parse_side(s: Option<&str>) -> Side {
    match s {
        Some("buy" | "Buy" | "BUY") => Side::Buy,
        _ => Side::Sell,
    }
}

/// Convert nanosecond timestamp to milliseconds.
pub fn ns_to_ms(ns: u64) -> u64 {
    if ns > 1_000_000_000_000_000 {
        // Nanoseconds
        ns / 1_000_000
    } else if ns > 1_000_000_000_000 {
        // Already milliseconds
        ns
    } else {
        // Seconds
        ns * 1000
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
    fn test_unified_to_kucoin_futures() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_kucoin_futures(&sym), "XBTUSDTM");

        let sym2 = Symbol::new("ETH", "USDT");
        assert_eq!(unified_to_kucoin_futures(&sym2), "ETHUSDTM");
    }

    #[test]
    fn test_kucoin_futures_symbol_to_unified() {
        let sym = kucoin_futures_symbol_to_unified("BTCUSDTM");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = kucoin_futures_symbol_to_unified("XBTUSDTM");
        assert_eq!(sym2.base, "BTC");
        assert_eq!(sym2.quote, "USDT");

        let sym3 = kucoin_futures_symbol_to_unified("ETHUSDTM");
        assert_eq!(sym3.base, "ETH");
        assert_eq!(sym3.quote, "USDT");

        let sym4 = kucoin_futures_symbol_to_unified("XBTUSDM");
        assert_eq!(sym4.base, "BTC");
        assert_eq!(sym4.quote, "USD");
    }

    #[test]
    fn test_kucoin_futures_pair_to_unified() {
        let sym = kucoin_futures_pair_to_unified("XBT", "USDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = kucoin_futures_pair_to_unified("ETH", "USDT");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDT");
    }

    #[test]
    fn test_interval_to_granularity() {
        assert_eq!(interval_to_granularity(Interval::M1), 1);
        assert_eq!(interval_to_granularity(Interval::M5), 5);
        assert_eq!(interval_to_granularity(Interval::M15), 15);
        assert_eq!(interval_to_granularity(Interval::H1), 60);
        assert_eq!(interval_to_granularity(Interval::H4), 240);
        assert_eq!(interval_to_granularity(Interval::D1), 1440);
        assert_eq!(interval_to_granularity(Interval::W1), 10080);
    }

    #[test]
    fn test_ns_to_ms() {
        // Nanoseconds
        assert_eq!(ns_to_ms(1700000000000000000), 1700000000000);
        // Already milliseconds
        assert_eq!(ns_to_ms(1700000000000), 1700000000000);
        // Seconds
        assert_eq!(ns_to_ms(1700000000), 1700000000000);
    }

    #[test]
    fn test_parse_levels_mixed_numbers() {
        let raw = vec![
            vec![
                serde_json::json!(50001.5),
                serde_json::json!(100),
            ],
            vec![
                serde_json::json!(50000.0),
                serde_json::json!(200),
            ],
        ];
        let levels = parse_levels_mixed(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(50001.5));
        assert_eq!(levels[0].qty, dec!(100));
    }

    #[test]
    fn test_parse_levels_mixed_strings() {
        let raw = vec![vec![
            serde_json::json!("50001.5"),
            serde_json::json!("100"),
        ]];
        let levels = parse_levels_mixed(&raw);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, dec!(50001.5));
    }

    #[test]
    fn test_parse_value_decimal() {
        assert_eq!(
            parse_value_decimal(Some(&serde_json::json!("0.0001"))),
            dec!(0.0001)
        );
        assert_eq!(
            parse_value_decimal(Some(&serde_json::json!(0.0001))),
            dec!(0.0001)
        );
        assert_eq!(parse_value_decimal(None), Decimal::ZERO);
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw = KfOrderBookRaw {
            symbol: Some("XBTUSDTM".into()),
            sequence: Some(1234567),
            asks: vec![vec![serde_json::json!(89131.0), serde_json::json!(50)]],
            bids: vec![vec![serde_json::json!(89130.0), serde_json::json!(100)]],
            ts: Some(1700000000000000000),
        };

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::KucoinFutures);
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(89130.0));
        assert_eq!(ob.asks[0].price, dec!(89131.0));
        assert_eq!(ob.sequence, Some(1234567));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_conversion() {
        let raw = KfTradeRaw {
            sequence: Some(1234567),
            trade_id: Some("abc123".into()),
            side: Some("buy".into()),
            price: Some("89131.0".into()),
            size: Some(serde_json::json!(10)),
            ts: Some(1700000000000000000),
        };

        let trade = raw.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::KucoinFutures);
        assert_eq!(trade.price, dec!(89131.0));
        assert_eq!(trade.qty, dec!(10));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("abc123".into()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_funding_rate_conversion() {
        let raw = KfFundingRateRaw {
            symbol: Some("XBTUSDTM".into()),
            granularity: Some(28800000),
            time_point: Some(1700000000000),
            value: Some(serde_json::json!(0.0001)),
            predicted_value: Some(serde_json::json!(0.00015)),
        };

        let sym = Symbol::new("BTC", "USDT");
        let fr = raw.into_funding_rate(&sym);
        assert_eq!(fr.exchange, ExchangeId::KucoinFutures);
        assert_eq!(fr.rate, dec!(0.0001));
        assert_eq!(fr.next_funding_time_ms, 1700028800000);
        assert_eq!(fr.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_mark_price_conversion() {
        let raw = KfMarkPriceRaw {
            symbol: Some("XBTUSDTM".into()),
            time_point: Some(1700000000000),
            value: Some(serde_json::json!(89131.36)),
            index_price: Some(serde_json::json!(89148.12)),
        };

        let sym = Symbol::new("BTC", "USDT");
        let mp = raw.into_mark_price(&sym);
        assert_eq!(mp.exchange, ExchangeId::KucoinFutures);
        assert_eq!(mp.mark_price, dec!(89131.36));
        assert_eq!(mp.index_price, dec!(89148.12));
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw = KfWsTradeData {
            symbol: Some("XBTUSDTM".into()),
            side: Some("buy".into()),
            price: Some("89131.0".into()),
            size: Some(serde_json::json!(10)),
            trade_id: Some("trade123".into()),
            ts: Some(1700000000000000000),
        };

        let trade = raw.into_trade();
        assert_eq!(trade.symbol.base, "BTC");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(89131.0));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_contract_into_symbol_info() {
        let raw = KfContractRaw {
            symbol: "XBTUSDTM".into(),
            base_currency: Some("XBT".into()),
            quote_currency: Some("USDT".into()),
            tick_size: Some(0.1),
            lot_size: Some(1.0),
            multiplier: Some(0.001),
            status: Some("Open".into()),
            mark_price: Some(89131.36),
            index_price: Some(89148.12),
            last_trade_price: Some(89126.5),
            open_interest: Some("4955514".into()),
            turnover_of24h: None,
            volume_of24h: None,
            price_change_pct_of24h: None,
            max_price: None,
            low_price: None,
        };

        let info = raw.into_symbol_info();
        assert_eq!(info.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.raw_symbol, "XBTUSDTM");
        assert_eq!(info.status, SymbolStatus::Trading);
        assert_eq!(info.tick_size, Some(dec!(0.1)));
    }
}
