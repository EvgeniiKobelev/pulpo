use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Symbol / Interval helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to an Asterdex raw symbol string (e.g. "BTCUSDT").
pub fn unified_to_asterdex(symbol: &Symbol) -> String {
    format!("{}{}", symbol.base, symbol.quote)
}

/// Known quote assets in priority order (longest first to avoid partial matches).
const KNOWN_QUOTES: &[&str] = &[
    "USDT", "USDC", "BTC", "ETH", "BNB", "EUR",
];

/// Convert an Asterdex raw symbol string (e.g. "BTCUSDT") to a unified Symbol.
pub fn asterdex_symbol_to_unified(raw: &str) -> Symbol {
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

/// Map a unified Interval to the Asterdex interval string.
pub fn interval_to_asterdex(interval: Interval) -> &'static str {
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
// Exchange Info
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexExchangeInfoRaw {
    pub symbols: Vec<AsterdexSymbolRaw>,
}

#[derive(Debug, Deserialize)]
pub struct AsterdexSymbolRaw {
    pub symbol: String,
    pub status: String,
    #[serde(rename = "baseAsset")]
    pub base_asset: String,
    #[serde(rename = "quoteAsset")]
    pub quote_asset: String,
    #[serde(rename = "pricePrecision")]
    pub price_precision: u8,
    #[serde(rename = "quantityPrecision")]
    pub quantity_precision: u8,
    #[serde(default)]
    pub filters: Vec<serde_json::Value>,
}

impl AsterdexExchangeInfoRaw {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .symbols
            .into_iter()
            .map(|s| {
                let status = match s.status.as_str() {
                    "TRADING" => SymbolStatus::Trading,
                    "HALT" => SymbolStatus::Halted,
                    "PRE_TRADING" => SymbolStatus::PreTrading,
                    _ => SymbolStatus::Unknown,
                };

                let mut min_qty: Option<Decimal> = None;
                let mut tick_size: Option<Decimal> = None;
                let mut min_notional: Option<Decimal> = None;

                for f in &s.filters {
                    let filter_type = f
                        .get("filterType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match filter_type {
                        "LOT_SIZE" => {
                            min_qty = f
                                .get("minQty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        "PRICE_FILTER" => {
                            tick_size = f
                                .get("tickSize")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        "MIN_NOTIONAL" => {
                            min_notional = f
                                .get("notional")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Decimal::from_str(s).ok());
                        }
                        _ => {}
                    }
                }

                SymbolInfo {
                    symbol: Symbol::new(&s.base_asset, &s.quote_asset),
                    raw_symbol: s.symbol,
                    status,
                    base_precision: s.price_precision,
                    quote_precision: s.quantity_precision,
                    min_qty,
                    min_notional,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::AsterdexFutures,
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// REST OrderBook
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexOrderBookRaw {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "T")]
    pub transaction_time: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

impl AsterdexOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.event_time,
            sequence: Some(self.last_update_id),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Trade
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexTradeRaw {
    pub id: u64,
    pub price: String,
    pub qty: String,
    pub time: u64,
    #[serde(rename = "isBuyerMaker")]
    pub is_buyer_maker: bool,
}

impl AsterdexTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        Trade {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker {
                Side::Sell
            } else {
                Side::Buy
            },
            timestamp_ms: self.time,
            trade_id: Some(self.id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Ticker
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "bidPrice", default)]
    pub bid_price: Option<String>,
    #[serde(rename = "askPrice", default)]
    pub ask_price: Option<String>,
    pub volume: String,
    #[serde(rename = "priceChangePercent")]
    pub price_change_percent: String,
    #[serde(rename = "closeTime")]
    pub close_time: u64,
}

impl AsterdexTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            last_price: Decimal::from_str(&self.last_price).unwrap_or_default(),
            bid: self.bid_price.as_deref().and_then(|s| Decimal::from_str(s).ok()),
            ask: self.ask_price.as_deref().and_then(|s| Decimal::from_str(s).ok()),
            volume_24h: Decimal::from_str(&self.volume).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.price_change_percent).ok(),
            timestamp_ms: self.close_time,
        }
    }
}

// ---------------------------------------------------------------------------
// Premium Index (funding rate + mark price)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexPremiumIndexRaw {
    pub symbol: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
    #[serde(rename = "indexPrice")]
    pub index_price: String,
    #[serde(rename = "lastFundingRate")]
    pub last_funding_rate: String,
    #[serde(rename = "nextFundingTime")]
    pub next_funding_time: u64,
    pub time: u64,
}

impl AsterdexPremiumIndexRaw {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        FundingRate {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            rate: Decimal::from_str(&self.last_funding_rate).unwrap_or_default(),
            next_funding_time_ms: self.next_funding_time,
            timestamp_ms: self.time,
        }
    }

    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.time,
        }
    }
}

// ---------------------------------------------------------------------------
// Open Interest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexOpenInterestRaw {
    pub symbol: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
    pub time: u64,
}

impl AsterdexOpenInterestRaw {
    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        let oi = Decimal::from_str(&self.open_interest).unwrap_or_default();
        OpenInterest {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            open_interest: oi,
            open_interest_value: Decimal::ZERO,
            timestamp_ms: self.time,
        }
    }
}

// ---------------------------------------------------------------------------
// Force Orders (Liquidations)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexForceOrderRaw {
    pub symbol: String,
    pub price: String,
    #[serde(rename = "origQty")]
    pub orig_qty: String,
    pub side: String,
    pub time: u64,
}

impl AsterdexForceOrderRaw {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "BUY" => Side::Buy,
            _ => Side::Sell,
        };
        Liquidation {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            side,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.orig_qty).unwrap_or_default(),
            timestamp_ms: self.time,
        }
    }
}

// ---------------------------------------------------------------------------
// WS Depth
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexWsDepthRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub bids: Vec<[String; 2]>,
    #[serde(rename = "a")]
    pub asks: Vec<[String; 2]>,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "u")]
    pub last_update_id: u64,
}

impl AsterdexWsDepthRaw {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        OrderBook {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.event_time,
            sequence: Some(self.last_update_id),
        }
    }
}

// ---------------------------------------------------------------------------
// WS Trade
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexWsTradeRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub qty: String,
    #[serde(rename = "T")]
    pub trade_time: u64,
    /// Aggregate trade ID (`a` field in aggTrade stream).
    #[serde(rename = "a")]
    pub agg_trade_id: u64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

impl AsterdexWsTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        Trade {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.qty).unwrap_or_default(),
            side: if self.is_buyer_maker {
                Side::Sell
            } else {
                Side::Buy
            },
            timestamp_ms: self.trade_time,
            trade_id: Some(self.agg_trade_id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// WS Kline
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexWsKlineMsg {
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    pub k: AsterdexWsKlineRaw,
}

#[derive(Debug, Deserialize)]
pub struct AsterdexWsKlineRaw {
    #[serde(rename = "t")]
    pub open_time: u64,
    #[serde(rename = "T")]
    pub close_time: u64,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "x")]
    pub is_closed: bool,
}

impl AsterdexWsKlineMsg {
    pub fn into_candle(self) -> Candle {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        Candle {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            open: Decimal::from_str(&self.k.open).unwrap_or_default(),
            high: Decimal::from_str(&self.k.high).unwrap_or_default(),
            low: Decimal::from_str(&self.k.low).unwrap_or_default(),
            close: Decimal::from_str(&self.k.close).unwrap_or_default(),
            volume: Decimal::from_str(&self.k.volume).unwrap_or_default(),
            open_time_ms: self.k.open_time,
            close_time_ms: self.k.close_time,
            is_closed: self.k.is_closed,
        }
    }
}

// ---------------------------------------------------------------------------
// WS Mark Price
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexWsMarkPriceRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub mark_price: String,
    #[serde(rename = "i")]
    pub index_price: String,
    #[serde(rename = "r")]
    pub funding_rate: String,
    #[serde(rename = "T")]
    pub next_funding_time: u64,
    #[serde(rename = "E")]
    pub event_time: u64,
}

impl AsterdexWsMarkPriceRaw {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = asterdex_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.event_time,
        }
    }
}

// ---------------------------------------------------------------------------
// WS Force Order (Liquidation)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexWsForceOrderMsg {
    #[serde(rename = "E")]
    pub event_time: u64,
    pub o: AsterdexWsForceOrderRaw,
}

#[derive(Debug, Deserialize)]
pub struct AsterdexWsForceOrderRaw {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "S")]
    pub side: String,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub qty: String,
    #[serde(rename = "T")]
    pub trade_time: u64,
}

impl AsterdexWsForceOrderMsg {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = asterdex_symbol_to_unified(&self.o.symbol);
        let side = match self.o.side.as_str() {
            "BUY" => Side::Buy,
            _ => Side::Sell,
        };
        Liquidation {
            exchange: ExchangeId::AsterdexFutures,
            symbol,
            side,
            price: Decimal::from_str(&self.o.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.o.qty).unwrap_or_default(),
            timestamp_ms: self.o.trade_time,
        }
    }
}

// ---------------------------------------------------------------------------
// Combined stream wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsterdexCombinedStream {
    pub stream: String,
    pub data: serde_json::Value,
}

// ---------------------------------------------------------------------------
// REST Kline parsing
// ---------------------------------------------------------------------------

/// Parse a single Asterdex kline array row into a Candle.
///
/// Asterdex returns klines as arrays (Binance-compatible):
/// `[openTime, open, high, low, close, volume, closeTime, ...]`
pub fn parse_kline_row(row: &[serde_json::Value], symbol: Symbol) -> Option<Candle> {
    Some(Candle {
        exchange: ExchangeId::AsterdexFutures,
        symbol,
        open: Decimal::from_str(row.get(1)?.as_str()?).ok()?,
        high: Decimal::from_str(row.get(2)?.as_str()?).ok()?,
        low: Decimal::from_str(row.get(3)?.as_str()?).ok()?,
        close: Decimal::from_str(row.get(4)?.as_str()?).ok()?,
        volume: Decimal::from_str(row.get(5)?.as_str()?).ok()?,
        open_time_ms: row.first()?.as_u64()?,
        close_time_ms: row.get(6)?.as_u64()?,
        is_closed: true,
    })
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub fn parse_levels(raw: &[[String; 2]]) -> Vec<Level> {
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
    fn test_unified_to_asterdex() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_asterdex(&sym), "BTCUSDT");
    }

    #[test]
    fn test_asterdex_symbol_to_unified() {
        let sym = asterdex_symbol_to_unified("BTCUSDT");
        assert_eq!(sym.base, "BTC");
        assert_eq!(sym.quote, "USDT");

        let sym2 = asterdex_symbol_to_unified("ETHUSDC");
        assert_eq!(sym2.base, "ETH");
        assert_eq!(sym2.quote, "USDC");
    }

    #[test]
    fn test_interval_to_asterdex() {
        assert_eq!(interval_to_asterdex(Interval::M1), "1m");
        assert_eq!(interval_to_asterdex(Interval::H1), "1h");
        assert_eq!(interval_to_asterdex(Interval::D1), "1d");
    }

    #[test]
    fn test_premium_index_into_funding_rate() {
        let raw = AsterdexPremiumIndexRaw {
            symbol: "BTCUSDT".to_string(),
            mark_price: "50000.00".to_string(),
            index_price: "49990.00".to_string(),
            last_funding_rate: "0.0001".to_string(),
            next_funding_time: 1700000000000,
            time: 1699999990000,
        };
        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(fr.rate, dec!(0.0001));
        assert_eq!(fr.next_funding_time_ms, 1700000000000);
    }

    #[test]
    fn test_premium_index_into_mark_price() {
        let raw = AsterdexPremiumIndexRaw {
            symbol: "ETHUSDT".to_string(),
            mark_price: "2000.50".to_string(),
            index_price: "2000.00".to_string(),
            last_funding_rate: "0.0002".to_string(),
            next_funding_time: 1700000000000,
            time: 1699999990000,
        };
        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(mp.mark_price, dec!(2000.50));
        assert_eq!(mp.index_price, dec!(2000.00));
    }

    #[test]
    fn test_open_interest_conversion() {
        let raw = AsterdexOpenInterestRaw {
            symbol: "BTCUSDT".to_string(),
            open_interest: "12345.678".to_string(),
            time: 1700000000000,
        };
        let oi = raw.into_open_interest();
        assert_eq!(oi.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(oi.open_interest, dec!(12345.678));
        assert_eq!(oi.open_interest_value, Decimal::ZERO);
    }

    #[test]
    fn test_force_order_into_liquidation() {
        let raw = AsterdexForceOrderRaw {
            symbol: "BTCUSDT".to_string(),
            price: "50000.00".to_string(),
            orig_qty: "0.5".to_string(),
            side: "BUY".to_string(),
            time: 1700000000000,
        };
        let liq = raw.into_liquidation();
        assert_eq!(liq.side, Side::Buy);
        assert_eq!(liq.price, dec!(50000.00));
        assert_eq!(liq.qty, dec!(0.5));
    }

    #[test]
    fn test_ws_mark_price_conversion() {
        let raw: AsterdexWsMarkPriceRaw = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "p": "50123.45",
                "i": "50100.00",
                "r": "0.00015",
                "T": 1700000000000,
                "E": 1699999999000
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(mp.mark_price, dec!(50123.45));
        assert_eq!(mp.index_price, dec!(50100.00));
    }

    #[test]
    fn test_ws_force_order_conversion() {
        let raw: AsterdexWsForceOrderMsg = serde_json::from_str(
            r#"{
                "E": 1700000000000,
                "o": {
                    "s": "BTCUSDT",
                    "S": "SELL",
                    "p": "48000.00",
                    "q": "1.5",
                    "T": 1700000000100
                }
            }"#,
        )
        .unwrap();

        let liq = raw.into_liquidation();
        assert_eq!(liq.side, Side::Sell);
        assert_eq!(liq.price, dec!(48000.00));
        assert_eq!(liq.qty, dec!(1.5));
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

    #[test]
    fn test_exchange_info_conversion() {
        let raw: AsterdexExchangeInfoRaw = serde_json::from_str(
            r#"{
                "symbols": [{
                    "symbol": "BTCUSDT",
                    "status": "TRADING",
                    "baseAsset": "BTC",
                    "quoteAsset": "USDT",
                    "pricePrecision": 2,
                    "quantityPrecision": 3,
                    "filters": [
                        {"filterType": "LOT_SIZE", "minQty": "0.001"},
                        {"filterType": "PRICE_FILTER", "tickSize": "0.10"},
                        {"filterType": "MIN_NOTIONAL", "notional": "5.0"}
                    ]
                }]
            }"#,
        )
        .unwrap();

        let info = raw.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(info.symbols.len(), 1);
        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.min_qty, Some(dec!(0.001)));
        assert_eq!(btc.tick_size, Some(dec!(0.10)));
        assert_eq!(btc.min_notional, Some(dec!(5.0)));
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw = AsterdexOrderBookRaw {
            last_update_id: 42,
            event_time: 1700000000000,
            transaction_time: 1700000000001,
            bids: vec![["50000.00".to_string(), "1.0".to_string()]],
            asks: vec![["50001.00".to_string(), "2.0".to_string()]],
        };
        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(42));
    }

    #[test]
    fn test_ticker_conversion_with_bid_ask() {
        let raw: AsterdexTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPrice": "50000.00",
                "bidPrice": "49999.00",
                "askPrice": "50001.00",
                "volume": "12345.678",
                "priceChangePercent": "2.5",
                "closeTime": 1700000000000
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
    }

    #[test]
    fn test_ticker_conversion_without_bid_ask() {
        let raw: AsterdexTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPrice": "66089.5",
                "volume": "20122.980",
                "priceChangePercent": "-1.422",
                "closeTime": 1772403886227
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(ticker.last_price, dec!(66089.5));
        assert_eq!(ticker.bid, None);
        assert_eq!(ticker.ask, None);
    }

    #[test]
    fn test_ws_depth_conversion() {
        let raw: AsterdexWsDepthRaw = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "b": [["50000.00", "1.0"]],
                "a": [["50001.00", "2.0"]],
                "E": 1700000000000,
                "u": 100
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(ob.symbol.base, "BTC");
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw: AsterdexWsTradeRaw = serde_json::from_str(
            r#"{
                "e": "aggTrade",
                "E": 1700000000000,
                "s": "ETHUSDT",
                "a": 71400283,
                "p": "2000.00",
                "q": "0.5",
                "f": 100,
                "l": 100,
                "T": 1700000000000,
                "m": false
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("71400283".to_string()));
    }

    #[test]
    fn test_ws_kline_conversion() {
        let raw: AsterdexWsKlineMsg = serde_json::from_str(
            r#"{
                "E": 1700000000000,
                "s": "BTCUSDT",
                "k": {
                    "t": 1700000000000,
                    "T": 1700000060000,
                    "o": "50000.00",
                    "c": "50100.00",
                    "h": "50200.00",
                    "l": "49900.00",
                    "v": "100.5",
                    "x": true
                }
            }"#,
        )
        .unwrap();

        let candle = raw.into_candle();
        assert_eq!(candle.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(candle.open, dec!(50000.00));
        assert!(candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row() {
        let row: Vec<serde_json::Value> = serde_json::from_str(
            r#"[1700000000000, "50000.00", "50200.00", "49900.00", "50100.00", "100.5", 1700000060000, "0", 0, "0", "0", "0"]"#,
        )
        .unwrap();

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::AsterdexFutures);
        assert_eq!(candle.open, dec!(50000.00));
    }
}
