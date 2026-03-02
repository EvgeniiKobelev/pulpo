use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to a BloFin instId (e.g. `Symbol::new("BTC","USDT")` -> `"BTC-USDT"`).
pub fn unified_to_blofin(symbol: &Symbol) -> String {
    format!("{}-{}", symbol.base, symbol.quote)
}

/// Convert a BloFin instId to a unified Symbol (e.g. `"BTC-USDT"` -> `Symbol::new("BTC","USDT")`).
pub fn blofin_symbol_to_unified(raw: &str) -> Symbol {
    if let Some((base, quote)) = raw.split_once('-') {
        Symbol::new(base, quote)
    } else {
        // Fallback: assume last 4 chars are the quote
        let mid = raw.len().saturating_sub(4);
        Symbol::new(&raw[..mid], &raw[mid..])
    }
}

/// Map a unified Interval to BloFin candle bar string.
///
/// BloFin uses: 1m, 3m, 5m, 15m, 30m, 1H, 4H, 1D, 1W
pub fn interval_to_blofin(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "1m", // BloFin doesn't support 1s, fallback to 1m
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

/// Map a unified Interval to BloFin WS candle channel suffix.
///
/// BloFin WS candle channels: candle1m, candle5m, candle1H, candle1D, etc.
pub fn interval_to_blofin_ws(interval: Interval) -> &'static str {
    interval_to_blofin(interval)
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/instruments
// ---------------------------------------------------------------------------

/// Top-level response from `GET /api/v1/market/instruments`.
#[derive(Debug, Deserialize)]
pub struct BlofinInstrumentsResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<BlofinInstrument>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinInstrument {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "baseCurrency", default)]
    pub base_currency: String,
    #[serde(rename = "quoteCurrency", default)]
    pub quote_currency: String,
    #[serde(rename = "tickSize", default)]
    pub tick_size: Option<String>,
    #[serde(rename = "lotSize", default)]
    pub lot_size: Option<String>,
    #[serde(rename = "minSize", default)]
    pub min_size: Option<String>,
    #[serde(rename = "contractValue", default)]
    pub contract_value: Option<String>,
    #[serde(rename = "maxLeverage", default)]
    pub max_leverage: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

impl BlofinInstrumentsResponse {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .data
            .into_iter()
            .filter(|i| i.state.as_deref() == Some("live"))
            .map(|i| {
                let symbol = blofin_symbol_to_unified(&i.inst_id);
                let tick_size = i.tick_size.as_deref().and_then(|s| Decimal::from_str(s).ok());
                let lot_size = i.lot_size.as_deref().and_then(|s| Decimal::from_str(s).ok());

                // Derive precision from tick/lot size
                let quote_precision = tick_size
                    .map(|d| d.scale() as u8)
                    .unwrap_or(2);
                let base_precision = lot_size
                    .map(|d| d.scale() as u8)
                    .unwrap_or(2);

                SymbolInfo {
                    raw_symbol: i.inst_id,
                    symbol,
                    status: SymbolStatus::Trading,
                    base_precision,
                    quote_precision,
                    min_qty: i.min_size.as_deref().and_then(|s| Decimal::from_str(s).ok()),
                    min_notional: None,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::BlofinFutures,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/books
// ---------------------------------------------------------------------------

/// Response from `GET /api/v1/market/books?instId=...`.
#[derive(Debug, Deserialize)]
pub struct BlofinOrderbookResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<BlofinOrderbookData>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinOrderbookData {
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
    pub ts: String,
}

impl BlofinOrderbookData {
    pub fn into_orderbook(self, inst_id: &str) -> OrderBook {
        let symbol = blofin_symbol_to_unified(inst_id);
        let timestamp_ms = self.ts.parse::<u64>().unwrap_or(0);
        OrderBook {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms,
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/trades
// ---------------------------------------------------------------------------

/// Response from `GET /api/v1/market/trades?instId=...`.
#[derive(Debug, Deserialize)]
pub struct BlofinTradesResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<BlofinTradeRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinTradeRaw {
    #[serde(rename = "tradeId", default)]
    pub trade_id: Option<String>,
    #[serde(rename = "instId")]
    pub inst_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub ts: String,
}

impl BlofinTradeRaw {
    pub fn into_trade(self) -> Option<Trade> {
        let symbol = blofin_symbol_to_unified(&self.inst_id);
        let side = match self.side.as_str() {
            "buy" => Side::Buy,
            _ => Side::Sell,
        };
        Some(Trade {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            price: Decimal::from_str(&self.price).ok()?,
            qty: Decimal::from_str(&self.size).ok()?,
            side,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            trade_id: self.trade_id,
        })
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/candles
// ---------------------------------------------------------------------------

/// Response from `GET /api/v1/market/candles?instId=...&bar=...`.
#[derive(Debug, Deserialize)]
pub struct BlofinCandlesResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<Vec<String>>,
}

impl BlofinCandlesResponse {
    /// Convert candle rows to Candles.
    ///
    /// Row format: `[ts, open, high, low, close, contractVol, baseVol, quoteVol, state]`
    pub fn into_candles(self, inst_id: &str, interval: Interval) -> Vec<Candle> {
        let symbol = blofin_symbol_to_unified(inst_id);
        let interval_ms = interval.as_secs() * 1000;
        self.data
            .into_iter()
            .filter_map(|row| {
                if row.len() < 7 {
                    return None;
                }
                let open_time_ms = row[0].parse::<u64>().ok()?;
                let open = Decimal::from_str(&row[1]).ok()?;
                let high = Decimal::from_str(&row[2]).ok()?;
                let low = Decimal::from_str(&row[3]).ok()?;
                let close = Decimal::from_str(&row[4]).ok()?;
                // row[5] = contract volume, row[6] = base volume
                let volume = Decimal::from_str(&row[6]).ok().unwrap_or_default();
                let is_closed = row.get(8).map(|s| s == "1").unwrap_or(true);

                Some(Candle {
                    exchange: ExchangeId::BlofinFutures,
                    symbol: symbol.clone(),
                    open,
                    high,
                    low,
                    close,
                    volume,
                    open_time_ms,
                    close_time_ms: open_time_ms + interval_ms,
                    is_closed,
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/tickers
// ---------------------------------------------------------------------------

/// Response from `GET /api/v1/market/tickers?instId=...`.
#[derive(Debug, Deserialize)]
pub struct BlofinTickersResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<BlofinTickerData>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinTickerData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(rename = "lastSize", default)]
    pub last_size: Option<String>,
    #[serde(rename = "askPrice", default)]
    pub ask_price: Option<String>,
    #[serde(rename = "askSize", default)]
    pub ask_size: Option<String>,
    #[serde(rename = "bidPrice", default)]
    pub bid_price: Option<String>,
    #[serde(rename = "bidSize", default)]
    pub bid_size: Option<String>,
    #[serde(rename = "high24h", default)]
    pub high_24h: Option<String>,
    #[serde(rename = "open24h", default)]
    pub open_24h: Option<String>,
    #[serde(rename = "low24h", default)]
    pub low_24h: Option<String>,
    #[serde(rename = "volCurrency24h", default)]
    pub vol_currency_24h: Option<String>,
    #[serde(rename = "vol24h", default)]
    pub vol_24h: Option<String>,
    pub ts: Option<String>,
}

impl BlofinTickerData {
    pub fn into_ticker(self) -> Ticker {
        let symbol = blofin_symbol_to_unified(&self.inst_id);
        let last_price = self
            .last
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or_default();
        let open = self
            .open_24h
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
            .ts
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        Ticker {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            last_price,
            bid: self
                .bid_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok()),
            ask: self
                .ask_price
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok()),
            volume_24h: self
                .vol_currency_24h
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            price_change_pct_24h: pct,
            timestamp_ms: ts,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/funding-rate
// ---------------------------------------------------------------------------

/// Response from `GET /api/v1/market/funding-rate?instId=...`.
#[derive(Debug, Deserialize)]
pub struct BlofinFundingRateResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<BlofinFundingRateData>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinFundingRateData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "fundingRate", default)]
    pub funding_rate: Option<String>,
    #[serde(rename = "fundingTime", default)]
    pub funding_time: Option<String>,
}

impl BlofinFundingRateData {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = blofin_symbol_to_unified(&self.inst_id);
        let next_funding_time_ms = self
            .funding_time
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        FundingRate {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            rate: self
                .funding_rate
                .as_deref()
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or_default(),
            next_funding_time_ms,
            timestamp_ms: now_ms(),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: /api/v1/market/mark-price
// ---------------------------------------------------------------------------

/// Response from `GET /api/v1/market/mark-price?instId=...`.
#[derive(Debug, Deserialize)]
pub struct BlofinMarkPriceResponse {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<BlofinMarkPriceData>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinMarkPriceData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "markPrice", default)]
    pub mark_price: Option<String>,
    #[serde(rename = "indexPrice", default)]
    pub index_price: Option<String>,
    pub ts: Option<String>,
}

impl BlofinMarkPriceData {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = blofin_symbol_to_unified(&self.inst_id);
        let ts = self
            .ts
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        MarkPrice {
            exchange: ExchangeId::BlofinFutures,
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
// WS: tickers channel (mark price)
// ---------------------------------------------------------------------------

/// WebSocket message from tickers channel.
#[derive(Debug, Deserialize)]
pub struct BlofinWsMessage {
    pub arg: Option<BlofinWsArg>,
    #[serde(default)]
    pub data: Vec<serde_json::Value>,
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BlofinWsArg {
    pub channel: String,
    #[serde(rename = "instId", default)]
    pub inst_id: Option<String>,
}

/// WebSocket ticker data (includes mark/index price).
#[derive(Debug, Deserialize)]
pub struct BlofinWsTickerData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(rename = "askPrice", default)]
    pub ask_price: Option<String>,
    #[serde(rename = "bidPrice", default)]
    pub bid_price: Option<String>,
    #[serde(rename = "high24h", default)]
    pub high_24h: Option<String>,
    #[serde(rename = "open24h", default)]
    pub open_24h: Option<String>,
    #[serde(rename = "low24h", default)]
    pub low_24h: Option<String>,
    #[serde(rename = "volCurrency24h", default)]
    pub vol_currency_24h: Option<String>,
    #[serde(rename = "vol24h", default)]
    pub vol_24h: Option<String>,
    #[serde(rename = "indexPrice", default)]
    pub index_price: Option<String>,
    #[serde(rename = "markPrice", default)]
    pub mark_price: Option<String>,
    pub ts: Option<String>,
}

impl BlofinWsTickerData {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = blofin_symbol_to_unified(&self.inst_id);
        let ts = self
            .ts
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        MarkPrice {
            exchange: ExchangeId::BlofinFutures,
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
// WS: orderbook channel
// ---------------------------------------------------------------------------

/// WebSocket orderbook data.
#[derive(Debug, Deserialize)]
pub struct BlofinWsOrderbookData {
    pub asks: Vec<Vec<String>>,
    pub bids: Vec<Vec<String>>,
    pub ts: String,
    #[serde(rename = "seqId", default)]
    pub seq_id: Option<u64>,
}

impl BlofinWsOrderbookData {
    pub fn into_orderbook(self, inst_id: &str) -> OrderBook {
        let symbol = blofin_symbol_to_unified(inst_id);
        let timestamp_ms = self.ts.parse::<u64>().unwrap_or(0);
        OrderBook {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms,
            sequence: self.seq_id,
        }
    }
}

// ---------------------------------------------------------------------------
// WS: trades channel
// ---------------------------------------------------------------------------

/// WebSocket trade data.
#[derive(Debug, Deserialize)]
pub struct BlofinWsTradeData {
    #[serde(rename = "tradeId", default)]
    pub trade_id: Option<String>,
    #[serde(rename = "instId")]
    pub inst_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub ts: String,
}

impl BlofinWsTradeData {
    pub fn into_trade(self) -> Option<Trade> {
        let symbol = blofin_symbol_to_unified(&self.inst_id);
        let side = match self.side.as_str() {
            "buy" => Side::Buy,
            _ => Side::Sell,
        };
        Some(Trade {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            price: Decimal::from_str(&self.price).ok()?,
            qty: Decimal::from_str(&self.size).ok()?,
            side,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            trade_id: self.trade_id,
        })
    }
}

// ---------------------------------------------------------------------------
// WS: candle channel
// ---------------------------------------------------------------------------

/// WebSocket candle data.
///
/// Row format: `[ts, open, high, low, close, contractVol, baseVol, quoteVol, state]`
#[derive(Debug, Deserialize)]
pub struct BlofinWsCandleData(pub Vec<String>);

impl BlofinWsCandleData {
    pub fn into_candle(self, inst_id: &str, interval: Interval) -> Option<Candle> {
        let row = self.0;
        if row.len() < 7 {
            return None;
        }
        let symbol = blofin_symbol_to_unified(inst_id);
        let open_time_ms = row[0].parse::<u64>().ok()?;
        let interval_ms = interval.as_secs() * 1000;
        let open = Decimal::from_str(&row[1]).ok()?;
        let high = Decimal::from_str(&row[2]).ok()?;
        let low = Decimal::from_str(&row[3]).ok()?;
        let close = Decimal::from_str(&row[4]).ok()?;
        let volume = Decimal::from_str(&row[6]).ok().unwrap_or_default();
        let is_closed = row.get(8).map(|s| s == "1").unwrap_or(false);

        Some(Candle {
            exchange: ExchangeId::BlofinFutures,
            symbol,
            open,
            high,
            low,
            close,
            volume,
            open_time_ms,
            close_time_ms: open_time_ms + interval_ms,
            is_closed,
        })
    }
}

// ---------------------------------------------------------------------------
// WS subscription response
// ---------------------------------------------------------------------------

/// Generic WebSocket response for subscribe/unsubscribe confirmations.
#[derive(Debug, Deserialize)]
pub struct BlofinWsResponse {
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub arg: Option<serde_json::Value>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub msg: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse price/size pairs from BloFin orderbook format.
///
/// BloFin returns `[[price, size, ...], ...]` — we only use the first two elements.
fn parse_levels(raw: &[Vec<String>]) -> Vec<Level> {
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
    fn test_unified_to_blofin() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_blofin(&sym), "BTC-USDT");
    }

    #[test]
    fn test_blofin_symbol_to_unified() {
        let sym = blofin_symbol_to_unified("BTC-USDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = blofin_symbol_to_unified("ETH-USDT");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDT");
    }

    #[test]
    fn test_instruments_into_exchange_info() {
        let raw: BlofinInstrumentsResponse = serde_json::from_str(
            r#"{
                "code": "0",
                "msg": "success",
                "data": [
                    {
                        "instId": "BTC-USDT",
                        "baseCurrency": "BTC",
                        "quoteCurrency": "USDT",
                        "tickSize": "0.1",
                        "lotSize": "0.001",
                        "minSize": "0.001",
                        "contractValue": "0.001",
                        "maxLeverage": "125",
                        "state": "live"
                    },
                    {
                        "instId": "DELISTED-USDT",
                        "baseCurrency": "DELISTED",
                        "quoteCurrency": "USDT",
                        "tickSize": "0.01",
                        "lotSize": "1",
                        "minSize": "1",
                        "state": "suspend"
                    }
                ]
            }"#,
        )
        .unwrap();

        let info = raw.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::BlofinFutures);
        assert_eq!(info.symbols.len(), 1); // suspend filtered out
        assert_eq!(info.symbols[0].symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.symbols[0].raw_symbol, "BTC-USDT");
        assert_eq!(info.symbols[0].tick_size, Some(dec!(0.1)));
        assert_eq!(info.symbols[0].min_qty, Some(dec!(0.001)));
    }

    #[test]
    fn test_orderbook_into_orderbook() {
        let raw: BlofinOrderbookResponse = serde_json::from_str(
            r#"{
                "code": "0",
                "msg": "success",
                "data": [
                    {
                        "asks": [["50001.5", "0.5", "0", "1"], ["50002.0", "1.0", "0", "2"]],
                        "bids": [["50000.0", "2.0", "0", "3"], ["49999.5", "1.5", "0", "1"]],
                        "ts": "1700000000000"
                    }
                ]
            }"#,
        )
        .unwrap();

        let ob = raw.data.into_iter().next().unwrap().into_orderbook("BTC-USDT");
        assert_eq!(ob.exchange, ExchangeId::BlofinFutures);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 2);
        assert_eq!(ob.bids[0].price, dec!(50000.0));
        assert_eq!(ob.bids[0].qty, dec!(2.0));
        assert_eq!(ob.asks[0].price, dec!(50001.5));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ticker_into_ticker() {
        let raw: BlofinTickersResponse = serde_json::from_str(
            r#"{
                "code": "0",
                "msg": "success",
                "data": [
                    {
                        "instId": "BTC-USDT",
                        "last": "50000.0",
                        "lastSize": "1",
                        "askPrice": "50001.5",
                        "askSize": "5",
                        "bidPrice": "49999.5",
                        "bidSize": "3",
                        "high24h": "51000.0",
                        "open24h": "49000.0",
                        "low24h": "48000.0",
                        "volCurrency24h": "12345.678",
                        "vol24h": "12345678",
                        "ts": "1700000000000"
                    }
                ]
            }"#,
        )
        .unwrap();

        let ticker = raw.data.into_iter().next().unwrap().into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::BlofinFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000.0));
        assert_eq!(ticker.bid, Some(dec!(49999.5)));
        assert_eq!(ticker.ask, Some(dec!(50001.5)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert!(ticker.price_change_pct_24h.is_some());
        assert_eq!(ticker.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_funding_rate_into_funding_rate() {
        let raw: BlofinFundingRateResponse = serde_json::from_str(
            r#"{
                "code": "0",
                "msg": "success",
                "data": [
                    {
                        "instId": "ETH-USDT",
                        "fundingRate": "0.000123",
                        "fundingTime": "1700003600000"
                    }
                ]
            }"#,
        )
        .unwrap();

        let fr = raw.data.into_iter().next().unwrap().into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::BlofinFutures);
        assert_eq!(fr.symbol, Symbol::new("ETH", "USDT"));
        assert_eq!(fr.rate, dec!(0.000123));
        assert_eq!(fr.next_funding_time_ms, 1700003600000);
    }

    #[test]
    fn test_mark_price_into_mark_price() {
        let raw: BlofinMarkPriceResponse = serde_json::from_str(
            r#"{
                "code": "0",
                "msg": "success",
                "data": [
                    {
                        "instId": "BTC-USDT",
                        "markPrice": "50123.5",
                        "indexPrice": "50100.0",
                        "ts": "1700000000000"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mp = raw.data.into_iter().next().unwrap().into_mark_price();
        assert_eq!(mp.mark_price, dec!(50123.5));
        assert_eq!(mp.index_price, dec!(50100.0));
    }

    #[test]
    fn test_candles_into_candles() {
        let raw: BlofinCandlesResponse = serde_json::from_str(
            r#"{
                "code": "0",
                "msg": "success",
                "data": [
                    ["1700000000000", "50000.0", "51000.0", "49000.0", "50500.0", "100", "1.5", "75000.0", "1"]
                ]
            }"#,
        )
        .unwrap();

        let candles = raw.into_candles("BTC-USDT", Interval::M1);
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].exchange, ExchangeId::BlofinFutures);
        assert_eq!(candles[0].symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(candles[0].open, dec!(50000.0));
        assert_eq!(candles[0].high, dec!(51000.0));
        assert_eq!(candles[0].low, dec!(49000.0));
        assert_eq!(candles[0].close, dec!(50500.0));
        assert_eq!(candles[0].volume, dec!(1.5));
        assert!(candles[0].is_closed);
    }

    #[test]
    fn test_parse_levels() {
        let raw = vec![
            vec!["100.50".to_string(), "1.5".to_string()],
            vec!["99.00".to_string(), "2.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(100.50));
        assert_eq!(levels[0].qty, dec!(1.5));
    }

    #[test]
    fn test_parse_levels_skips_invalid() {
        let raw = vec![
            vec!["bad".to_string(), "1.0".to_string()],
            vec!["50.00".to_string(), "3.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 1);
    }

    #[test]
    fn test_interval_to_blofin() {
        assert_eq!(interval_to_blofin(Interval::M1), "1m");
        assert_eq!(interval_to_blofin(Interval::M5), "5m");
        assert_eq!(interval_to_blofin(Interval::H1), "1H");
        assert_eq!(interval_to_blofin(Interval::H4), "4H");
        assert_eq!(interval_to_blofin(Interval::D1), "1D");
        assert_eq!(interval_to_blofin(Interval::W1), "1W");
    }

    #[test]
    fn test_ws_trade_data_into_trade() {
        let raw: BlofinWsTradeData = serde_json::from_str(
            r#"{
                "tradeId": "123456",
                "instId": "BTC-USDT",
                "price": "50000.5",
                "size": "0.1",
                "side": "buy",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade().unwrap();
        assert_eq!(trade.exchange, ExchangeId::BlofinFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
        assert_eq!(trade.qty, dec!(0.1));
        assert_eq!(trade.trade_id, Some("123456".to_string()));
    }

    #[test]
    fn test_ws_orderbook_data_into_orderbook() {
        let raw: BlofinWsOrderbookData = serde_json::from_str(
            r#"{
                "asks": [["20702.9", "0.718"]],
                "bids": [["20700.5", "1.622"]],
                "ts": "1666854171201",
                "seqId": 77668172
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook("BTC-USDT");
        assert_eq!(ob.exchange, ExchangeId::BlofinFutures);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.asks[0].price, dec!(20702.9));
        assert_eq!(ob.bids[0].qty, dec!(1.622));
        assert_eq!(ob.sequence, Some(77668172));
    }
}
