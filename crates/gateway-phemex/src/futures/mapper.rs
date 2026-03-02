use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to a Phemex symbol (e.g. `Symbol::new("BTC","USDT")` → `"BTCUSDT"`).
pub fn unified_to_phemex(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Convert a Phemex raw symbol to a unified Symbol (e.g. `"BTCUSDT"` → `Symbol::new("BTC","USDT")`).
///
/// Phemex V2 perpetuals are quoted in USDT or USDC. We match known quote
/// currencies by suffix.
pub fn phemex_symbol_to_unified(raw: &str) -> Symbol {
    const KNOWN_QUOTES: &[&str] = &["USDT", "USDC", "USD"];
    for quote in KNOWN_QUOTES {
        if raw.ends_with(quote) {
            let base = &raw[..raw.len() - quote.len()];
            if !base.is_empty() {
                return Symbol::new(base, *quote);
            }
        }
    }
    // Fallback: assume last 4 chars are the quote
    let mid = raw.len().saturating_sub(4);
    Symbol::new(&raw[..mid], &raw[mid..])
}

/// Map a unified Interval to Phemex kline resolution in seconds.
pub fn interval_to_phemex(interval: Interval) -> u64 {
    interval.as_secs()
}

/// Map a unified Interval to the Phemex kline WS interval string.
pub fn interval_to_phemex_ws(interval: Interval) -> u64 {
    interval.as_secs()
}

// ---------------------------------------------------------------------------
// REST: /public/products → perpProductsV2
// ---------------------------------------------------------------------------

/// Top-level response from `GET /public/products`.
#[derive(Debug, Deserialize)]
pub struct PhemexProductsResponse {
    pub code: i64,
    pub msg: String,
    pub data: PhemexProductsData,
}

#[derive(Debug, Deserialize)]
pub struct PhemexProductsData {
    #[serde(rename = "perpProductsV2", default)]
    pub perp_products_v2: Vec<PhemexPerpProduct>,
}

#[derive(Debug, Deserialize)]
pub struct PhemexPerpProduct {
    pub symbol: String,
    #[serde(rename = "baseCurrency")]
    pub base_currency: String,
    #[serde(rename = "quoteCurrency")]
    pub quote_currency: String,
    #[serde(rename = "settleCurrency")]
    pub settle_currency: String,
    pub status: String,
    #[serde(rename = "pricePrecision")]
    pub price_precision: u8,
    #[serde(rename = "qtyPrecision")]
    pub qty_precision: u8,
    #[serde(rename = "tickSize", default)]
    pub tick_size: Option<String>,
    #[serde(rename = "qtyStepSize", default)]
    pub qty_step_size: Option<String>,
    #[serde(rename = "minOrderValueRv", default)]
    pub min_order_value_rv: Option<String>,
    #[serde(rename = "maxLeverage", default)]
    pub max_leverage: Option<u32>,
    #[serde(rename = "maxOrderQtyRq", default)]
    pub max_order_qty_rq: Option<String>,
    #[serde(rename = "fundingInterval", default)]
    pub funding_interval: Option<u64>,
    #[serde(rename = "type", default)]
    pub product_type: Option<String>,
}

impl PhemexProductsData {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .perp_products_v2
            .into_iter()
            .filter(|p| p.status == "Listed")
            .map(|p| {
                let symbol = Symbol::new(&p.base_currency, &p.quote_currency);
                SymbolInfo {
                    raw_symbol: p.symbol,
                    symbol,
                    status: SymbolStatus::Trading,
                    base_precision: p.qty_precision,
                    quote_precision: p.price_precision,
                    min_qty: p
                        .qty_step_size
                        .as_deref()
                        .and_then(|s| Decimal::from_str(s).ok()),
                    min_notional: p
                        .min_order_value_rv
                        .as_deref()
                        .and_then(|s| Decimal::from_str(s).ok()),
                    tick_size: p
                        .tick_size
                        .as_deref()
                        .and_then(|s| Decimal::from_str(s).ok()),
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::PhemexFutures,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /md/v2/orderbook
// ---------------------------------------------------------------------------

/// Response from `GET /md/v2/orderbook?symbol=...`.
#[derive(Debug, Deserialize)]
pub struct PhemexOrderbookResponse {
    pub error: Option<serde_json::Value>,
    pub id: Option<i64>,
    pub result: PhemexOrderbookResult,
}

#[derive(Debug, Deserialize)]
pub struct PhemexOrderbookResult {
    #[serde(rename = "orderbook_p")]
    pub orderbook: PhemexOrderbookData,
    pub depth: u32,
    pub sequence: u64,
    pub symbol: String,
    pub timestamp: u64,
    #[serde(rename = "type")]
    pub msg_type: String,
}

#[derive(Debug, Deserialize)]
pub struct PhemexOrderbookData {
    pub asks: Vec<[String; 2]>,
    pub bids: Vec<[String; 2]>,
}

impl PhemexOrderbookResult {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        // Phemex timestamps are in nanoseconds
        let timestamp_ms = self.timestamp / 1_000_000;
        OrderBook {
            exchange: ExchangeId::PhemexFutures,
            symbol,
            bids: parse_levels(&self.orderbook.bids),
            asks: parse_levels(&self.orderbook.asks),
            timestamp_ms,
            sequence: Some(self.sequence),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /md/v2/trade
// ---------------------------------------------------------------------------

/// Response from `GET /md/v2/trade?symbol=...`.
#[derive(Debug, Deserialize)]
pub struct PhemexTradesResponse {
    pub error: Option<serde_json::Value>,
    pub id: Option<i64>,
    pub result: PhemexTradesResult,
}

#[derive(Debug, Deserialize)]
pub struct PhemexTradesResult {
    pub sequence: u64,
    pub symbol: String,
    pub trades_p: Vec<PhemexTradeRaw>,
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Each trade from the V2 API is an array: `[timestamp_ns, side, priceRp, qtyRq, ...]`.
///
/// Side values: "Buy" or "Sell".
#[derive(Debug, Deserialize)]
pub struct PhemexTradeRaw(pub Vec<serde_json::Value>);

impl PhemexTradeRaw {
    pub fn into_trade(self, symbol_str: &str) -> Option<Trade> {
        let arr = self.0;
        if arr.len() < 4 {
            return None;
        }
        let timestamp_ns = arr[0].as_u64()?;
        let side_str = arr[1].as_str()?;
        let price_str = arr[2].as_str()?;
        let qty_str = arr[3].as_str()?;

        let symbol = phemex_symbol_to_unified(symbol_str);
        let side = match side_str {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        };

        Some(Trade {
            exchange: ExchangeId::PhemexFutures,
            symbol,
            price: Decimal::from_str(price_str).ok()?,
            qty: Decimal::from_str(qty_str).ok()?,
            side,
            timestamp_ms: timestamp_ns / 1_000_000,
            trade_id: None,
        })
    }
}

// ---------------------------------------------------------------------------
// REST: /md/v2/kline
// ---------------------------------------------------------------------------

/// Response from `GET /md/v2/kline?symbol=...&interval=...&from=...&to=...`.
#[derive(Debug, Deserialize)]
pub struct PhemexKlineResponse {
    pub error: Option<serde_json::Value>,
    pub id: Option<i64>,
    pub result: PhemexKlineResult,
}

#[derive(Debug, Deserialize)]
pub struct PhemexKlineResult {
    /// Nanosecond timestamp of the response.
    #[serde(default)]
    pub dts: u64,
    /// Kline rows for V2 hedged perpetual.
    #[serde(rename = "kline_p", default)]
    pub kline_p: Vec<Vec<serde_json::Value>>,
}

impl PhemexKlineResult {
    /// Convert kline rows to Candles.
    ///
    /// Row format: `[openEpochSec, intervalSecs, lastCloseRp, openRp, highRp, lowRp, closeRp, volumeRq, turnoverRv]`
    pub fn into_candles(self) -> Vec<Candle> {
        self.kline_p
            .into_iter()
            .filter_map(|row| {
                if row.len() < 9 {
                    return None;
                }
                let open_time_s = row[0].as_u64()?;
                let interval_s = row[1].as_u64()?;
                // row[2] = lastCloseRp (previous candle's close price, skip)
                let open = Decimal::from_str(row[3].as_str()?).ok()?;
                let high = Decimal::from_str(row[4].as_str()?).ok()?;
                let low = Decimal::from_str(row[5].as_str()?).ok()?;
                let close = Decimal::from_str(row[6].as_str()?).ok()?;
                let volume = Decimal::from_str(row[7].as_str()?).ok()?;

                // Phemex kline endpoint returns the symbol from the query context,
                // not per-row. We derive it from the calling function.
                Some(Candle {
                    exchange: ExchangeId::PhemexFutures,
                    symbol: Symbol::new("", ""),  // placeholder, filled by caller
                    open,
                    high,
                    low,
                    close,
                    volume,
                    open_time_ms: open_time_s * 1000,
                    close_time_ms: (open_time_s + interval_s) * 1000,
                    is_closed: true,
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// REST: /md/v2/ticker/24hr
// ---------------------------------------------------------------------------

/// Response from `GET /md/v2/ticker/24hr?symbol=...`.
#[derive(Debug, Deserialize)]
pub struct PhemexTickerResponse {
    pub error: Option<serde_json::Value>,
    pub id: Option<i64>,
    pub result: PhemexTickerResult,
}

#[derive(Debug, Deserialize)]
pub struct PhemexTickerResult {
    pub symbol: String,
    #[serde(rename = "openRp", default)]
    pub open: Option<String>,
    #[serde(rename = "highRp", default)]
    pub high: Option<String>,
    #[serde(rename = "lowRp", default)]
    pub low: Option<String>,
    /// Last/close price. Phemex V2 uses `closeRp` for the latest price.
    #[serde(rename = "closeRp", default)]
    pub close: Option<String>,
    #[serde(rename = "volumeRq", default)]
    pub volume: Option<String>,
    #[serde(rename = "turnoverRv", default)]
    pub turnover: Option<String>,
    #[serde(rename = "fundingRateRr", default)]
    pub funding_rate: Option<String>,
    #[serde(rename = "markPriceRp", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "indexPriceRp", default)]
    pub index_price: Option<String>,
    #[serde(rename = "openInterestRv", default)]
    pub open_interest: Option<String>,
    #[serde(rename = "predFundingRateRr", default)]
    pub pred_funding_rate: Option<String>,
    #[serde(rename = "nextFundingTime", default)]
    pub next_funding_time: Option<u64>,
    pub timestamp: Option<u64>,
}

impl PhemexTickerResult {
    pub fn into_ticker(self) -> Ticker {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        let last_price = self
            .close
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let open = self
            .open
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok());

        let pct = open.and_then(|o| {
            if !o.is_zero() {
                Some(((last_price - o) / o) * Decimal::from(100))
            } else {
                None
            }
        });

        let ts = self
            .timestamp
            .map(|ns| ns / 1_000_000)
            .unwrap_or(0);

        Ticker {
            exchange: ExchangeId::PhemexFutures,
            symbol,
            last_price,
            bid: None,
            ask: None,
            volume_24h: self
                .volume
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            price_change_pct_24h: pct,
            timestamp_ms: ts,
        }
    }

    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        let ts = self.timestamp.map(|ns| ns / 1_000_000).unwrap_or(0);
        FundingRate {
            exchange: ExchangeId::PhemexFutures,
            symbol,
            rate: self
                .funding_rate
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            next_funding_time_ms: self.next_funding_time.unwrap_or(0),
            timestamp_ms: ts,
        }
    }

    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        let ts = self.timestamp.map(|ns| ns / 1_000_000).unwrap_or(0);
        MarkPrice {
            exchange: ExchangeId::PhemexFutures,
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
            timestamp_ms: ts,
        }
    }

    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        let ts = self.timestamp.map(|ns| ns / 1_000_000).unwrap_or(0);
        let oi = self
            .open_interest
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let mark = self
            .mark_price
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        OpenInterest {
            exchange: ExchangeId::PhemexFutures,
            symbol,
            open_interest: oi,
            open_interest_value: oi * mark,
            timestamp_ms: ts,
        }
    }
}

/// Response from `GET /md/v2/ticker/24hr/all`.
#[derive(Debug, Deserialize)]
pub struct PhemexAllTickersResponse {
    pub error: Option<serde_json::Value>,
    pub id: Option<i64>,
    pub result: Vec<PhemexTickerResult>,
}

// ---------------------------------------------------------------------------
// WS: orderbook_p
// ---------------------------------------------------------------------------

/// WebSocket orderbook_p message (both snapshot and incremental).
#[derive(Debug, Deserialize)]
pub struct PhemexWsOrderbookMsg {
    #[serde(rename = "orderbook_p")]
    pub orderbook: PhemexOrderbookData,
    pub depth: u32,
    pub sequence: u64,
    pub symbol: String,
    pub timestamp: u64,
    #[serde(rename = "type")]
    pub msg_type: String,
}

impl PhemexWsOrderbookMsg {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        let timestamp_ms = self.timestamp / 1_000_000;
        OrderBook {
            exchange: ExchangeId::PhemexFutures,
            symbol,
            bids: parse_levels(&self.orderbook.bids),
            asks: parse_levels(&self.orderbook.asks),
            timestamp_ms,
            sequence: Some(self.sequence),
        }
    }
}

// ---------------------------------------------------------------------------
// WS: trade_p
// ---------------------------------------------------------------------------

/// WebSocket trade_p message.
///
/// Phemex trade messages use `dts` / `mts` for timestamps (not `timestamp`),
/// and the `type` field may be absent on the initial snapshot.
#[derive(Debug, Deserialize)]
pub struct PhemexWsTradeMsg {
    pub trades_p: Vec<Vec<serde_json::Value>>,
    pub sequence: u64,
    pub symbol: String,
    /// Nanosecond timestamp (data timestamp).
    #[serde(default)]
    pub dts: u64,
    #[serde(rename = "type", default)]
    pub msg_type: Option<String>,
}

impl PhemexWsTradeMsg {
    pub fn into_trades(self) -> Vec<Trade> {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        self.trades_p
            .into_iter()
            .filter_map(|arr| {
                if arr.len() < 4 {
                    return None;
                }
                let timestamp_ns = arr[0].as_u64()?;
                let side_str = arr[1].as_str()?;
                let price_str = arr[2].as_str()?;
                let qty_str = arr[3].as_str()?;

                let side = match side_str {
                    "Buy" => Side::Buy,
                    _ => Side::Sell,
                };

                Some(Trade {
                    exchange: ExchangeId::PhemexFutures,
                    symbol: symbol.clone(),
                    price: Decimal::from_str(price_str).ok()?,
                    qty: Decimal::from_str(qty_str).ok()?,
                    side,
                    timestamp_ms: timestamp_ns / 1_000_000,
                    trade_id: None,
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// WS: kline_p
// ---------------------------------------------------------------------------

/// WebSocket kline_p message.
#[derive(Debug, Deserialize)]
pub struct PhemexWsKlineMsg {
    pub kline_p: Vec<Vec<serde_json::Value>>,
    pub sequence: u64,
    pub symbol: String,
    #[serde(default)]
    pub dts: u64,
    #[serde(rename = "type", default)]
    pub msg_type: Option<String>,
}

impl PhemexWsKlineMsg {
    pub fn into_candles(self) -> Vec<Candle> {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        self.kline_p
            .into_iter()
            .filter_map(|row| {
                if row.len() < 9 {
                    return None;
                }
                let open_time_s = row[0].as_u64()?;
                let interval_s = row[1].as_u64()?;
                // row[2] = lastCloseRp (previous candle's close, skip)
                let open = Decimal::from_str(row[3].as_str()?).ok()?;
                let high = Decimal::from_str(row[4].as_str()?).ok()?;
                let low = Decimal::from_str(row[5].as_str()?).ok()?;
                let close = Decimal::from_str(row[6].as_str()?).ok()?;
                let volume = Decimal::from_str(row[7].as_str()?).ok()?;

                Some(Candle {
                    exchange: ExchangeId::PhemexFutures,
                    symbol: symbol.clone(),
                    open,
                    high,
                    low,
                    close,
                    volume,
                    open_time_ms: open_time_s * 1000,
                    close_time_ms: (open_time_s + interval_s) * 1000,
                    is_closed: false,
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// WS: perp_market24h_pack_p (tick/mark price)
// ---------------------------------------------------------------------------

/// WebSocket tick message for mark price updates.
#[derive(Debug, Deserialize)]
pub struct PhemexWsTickMsg {
    pub symbol: String,
    #[serde(rename = "markPriceRp", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "indexPriceRp", default)]
    pub index_price: Option<String>,
    #[serde(rename = "fundingRateRr", default)]
    pub funding_rate: Option<String>,
    #[serde(rename = "openInterestRv", default)]
    pub open_interest: Option<String>,
    pub timestamp: Option<u64>,
}

impl PhemexWsTickMsg {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = phemex_symbol_to_unified(&self.symbol);
        let ts = self.timestamp.map(|ns| ns / 1_000_000).unwrap_or(0);
        MarkPrice {
            exchange: ExchangeId::PhemexFutures,
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
            timestamp_ms: ts,
        }
    }
}

// ---------------------------------------------------------------------------
// WS subscription response
// ---------------------------------------------------------------------------

/// Generic WebSocket response for subscribe/pong.
#[derive(Debug, Deserialize)]
pub struct PhemexWsResponse {
    pub error: Option<serde_json::Value>,
    pub id: Option<i64>,
    pub result: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Helpers
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_unified_to_phemex() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_phemex(&sym), "BTCUSDT");
    }

    #[test]
    fn test_phemex_symbol_to_unified() {
        let sym = phemex_symbol_to_unified("BTCUSDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = phemex_symbol_to_unified("ETHUSDC");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDC");
    }

    #[test]
    fn test_products_into_exchange_info() {
        let raw: PhemexProductsResponse = serde_json::from_str(
            r#"{
                "code": 0,
                "msg": "",
                "data": {
                    "perpProductsV2": [
                        {
                            "symbol": "BTCUSDT",
                            "baseCurrency": "BTC",
                            "quoteCurrency": "USDT",
                            "settleCurrency": "USDT",
                            "status": "Listed",
                            "pricePrecision": 1,
                            "qtyPrecision": 3,
                            "tickSize": "0.1",
                            "qtyStepSize": "0.001",
                            "minOrderValueRv": "1",
                            "maxLeverage": 100,
                            "fundingInterval": 28800,
                            "type": "PerpetualV2"
                        },
                        {
                            "symbol": "DELISTEDUSDT",
                            "baseCurrency": "DELISTED",
                            "quoteCurrency": "USDT",
                            "settleCurrency": "USDT",
                            "status": "Delisted",
                            "pricePrecision": 4,
                            "qtyPrecision": 1
                        }
                    ]
                }
            }"#,
        )
        .unwrap();

        let info = raw.data.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::PhemexFutures);
        assert_eq!(info.symbols.len(), 1); // Delisted filtered out
        assert_eq!(info.symbols[0].symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.symbols[0].raw_symbol, "BTCUSDT");
        assert_eq!(info.symbols[0].base_precision, 3);
        assert_eq!(info.symbols[0].quote_precision, 1);
        assert_eq!(info.symbols[0].tick_size, Some(dec!(0.1)));
        assert_eq!(info.symbols[0].min_qty, Some(dec!(0.001)));
        assert_eq!(info.symbols[0].min_notional, Some(dec!(1)));
    }

    #[test]
    fn test_orderbook_result_into_orderbook() {
        let raw: PhemexOrderbookResponse = serde_json::from_str(
            r#"{
                "error": null,
                "id": 0,
                "result": {
                    "orderbook_p": {
                        "asks": [["50001.5", "0.5"], ["50002.0", "1.0"]],
                        "bids": [["50000.0", "2.0"], ["49999.5", "1.5"]]
                    },
                    "depth": 30,
                    "sequence": 12345,
                    "symbol": "BTCUSDT",
                    "timestamp": 1700000000000000000,
                    "type": "snapshot"
                }
            }"#,
        )
        .unwrap();

        let ob = raw.result.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::PhemexFutures);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 2);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.bids[0].qty, dec!(2.0));
        assert_eq!(ob.asks[0].price, dec!(50001.5));
        assert_eq!(ob.sequence, Some(12345));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ticker_into_ticker() {
        let raw: PhemexTickerResponse = serde_json::from_str(
            r#"{
                "error": null,
                "id": 0,
                "result": {
                    "symbol": "BTCUSDT",
                    "openRp": "49000.0",
                    "highRp": "51000.0",
                    "lowRp": "48000.0",
                    "closeRp": "50000.0",
                    "volumeRq": "12345.678",
                    "turnoverRv": "600000000.0",
                    "fundingRateRr": "0.0001",
                    "markPriceRp": "50000.5",
                    "indexPriceRp": "50000.0",
                    "openInterestRv": "5000.0",
                    "nextFundingTime": 1700003600000,
                    "timestamp": 1700000000000000000
                }
            }"#,
        )
        .unwrap();

        let ticker = raw.result.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::PhemexFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000.0));
        assert_eq!(ticker.bid, None);
        assert_eq!(ticker.ask, None);
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert!(ticker.price_change_pct_24h.is_some());
        assert_eq!(ticker.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ticker_into_funding_rate() {
        let raw: PhemexTickerResponse = serde_json::from_str(
            r#"{
                "error": null,
                "id": 0,
                "result": {
                    "symbol": "ETHUSDT",
                    "closeRp": "2000.0",
                    "fundingRateRr": "0.000123",
                    "markPriceRp": "2000.0",
                    "indexPriceRp": "2000.0",
                    "nextFundingTime": 1700003600000,
                    "timestamp": 1700000000000000000
                }
            }"#,
        )
        .unwrap();

        let fr = raw.result.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::PhemexFutures);
        assert_eq!(fr.symbol, Symbol::new("ETH", "USDT"));
        assert_eq!(fr.rate, dec!(0.000123));
        assert_eq!(fr.next_funding_time_ms, 1700003600000);
    }

    #[test]
    fn test_ticker_into_mark_price() {
        let raw: PhemexTickerResponse = serde_json::from_str(
            r#"{
                "error": null,
                "id": 0,
                "result": {
                    "symbol": "BTCUSDT",
                    "closeRp": "50123.5",
                    "markPriceRp": "50123.5",
                    "indexPriceRp": "50100.0",
                    "timestamp": 1700000000000000000
                }
            }"#,
        )
        .unwrap();

        let mp = raw.result.into_mark_price();
        assert_eq!(mp.mark_price, dec!(50123.5));
        assert_eq!(mp.index_price, dec!(50100.0));
    }

    #[test]
    fn test_ticker_into_open_interest() {
        let raw: PhemexTickerResponse = serde_json::from_str(
            r#"{
                "error": null,
                "id": 0,
                "result": {
                    "symbol": "BTCUSDT",
                    "closeRp": "50000.0",
                    "markPriceRp": "50000.0",
                    "openInterestRv": "100.5",
                    "timestamp": 1700000000000000000
                }
            }"#,
        )
        .unwrap();

        let oi = raw.result.into_open_interest();
        assert_eq!(oi.open_interest, dec!(100.5));
        assert_eq!(oi.open_interest_value, dec!(100.5) * dec!(50000.0));
    }

    #[test]
    fn test_ws_orderbook_into_orderbook() {
        let raw: PhemexWsOrderbookMsg = serde_json::from_str(
            r#"{
                "orderbook_p": {
                    "asks": [["20702.9", "0.718"]],
                    "bids": [["20700.5", "1.622"]]
                },
                "depth": 30,
                "sequence": 77668172,
                "symbol": "BTCUSDT",
                "timestamp": 1666854171201355264,
                "type": "snapshot"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::PhemexFutures);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.asks[0].price, dec!(20702.9));
        assert_eq!(ob.bids[0].qty, dec!(1.622));
        assert_eq!(ob.sequence, Some(77668172));
    }

    #[test]
    fn test_ws_trade_msg_into_trades() {
        let raw: PhemexWsTradeMsg = serde_json::from_str(
            r#"{
                "trades_p": [
                    [1666854171201355264, "Buy", "50000.5", "0.1"],
                    [1666854171301355264, "Sell", "49999.0", "0.5"]
                ],
                "sequence": 12345,
                "symbol": "BTCUSDT",
                "timestamp": 1666854171301355264,
                "type": "incremental"
            }"#,
        )
        .unwrap();

        let trades = raw.into_trades();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].side, Side::Buy);
        assert_eq!(trades[0].price, dec!(50000.5));
        assert_eq!(trades[0].qty, dec!(0.1));
        assert_eq!(trades[1].side, Side::Sell);
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
    }

    #[test]
    fn test_parse_levels_skips_invalid() {
        let raw = vec![
            ["bad".to_string(), "1.0".to_string()],
            ["50.00".to_string(), "3.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 1);
    }
}
