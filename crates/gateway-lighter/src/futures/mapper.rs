use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Market cache – maps between unified Symbol ↔ Lighter market_id
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LighterMarket {
    pub market_id: u16,
    pub symbol: Symbol,
    pub raw_symbol: String,
    pub size_decimals: u8,
    pub price_decimals: u8,
    pub min_base_amount: Option<Decimal>,
    pub min_quote_amount: Option<Decimal>,
}

#[derive(Debug, Clone, Default)]
pub struct MarketCache {
    pub by_id: HashMap<u16, LighterMarket>,
    pub by_symbol: HashMap<Symbol, LighterMarket>,
}

impl MarketCache {
    pub fn market_id(&self, symbol: &Symbol) -> Option<u16> {
        self.by_symbol.get(symbol).map(|m| m.market_id)
    }

    pub fn symbol(&self, market_id: u16) -> Option<Symbol> {
        self.by_id.get(&market_id).map(|m| m.symbol.clone())
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/orderBooks  (exchange info / market list)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterOrderBooksResponse {
    pub code: u16,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub order_books: Vec<LighterOrderBookMeta>,
}

#[derive(Debug, Deserialize)]
pub struct LighterOrderBookMeta {
    pub symbol: String,
    pub market_id: u16,
    pub market_type: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub taker_fee: String,
    #[serde(default)]
    pub maker_fee: String,
    #[serde(default)]
    pub min_base_amount: String,
    #[serde(default)]
    pub min_quote_amount: String,
    #[serde(default)]
    pub supported_size_decimals: u8,
    #[serde(default)]
    pub supported_price_decimals: u8,
}

impl LighterOrderBooksResponse {
    pub fn into_market_cache(&self) -> MarketCache {
        let mut cache = MarketCache::default();
        for meta in &self.order_books {
            if meta.market_type != "perp" {
                continue;
            }
            let symbol = lighter_symbol_to_unified(&meta.symbol);
            let market = LighterMarket {
                market_id: meta.market_id,
                symbol: symbol.clone(),
                raw_symbol: meta.symbol.clone(),
                size_decimals: meta.supported_size_decimals,
                price_decimals: meta.supported_price_decimals,
                min_base_amount: Decimal::from_str(&meta.min_base_amount).ok(),
                min_quote_amount: Decimal::from_str(&meta.min_quote_amount).ok(),
            };
            cache.by_id.insert(meta.market_id, market.clone());
            cache.by_symbol.insert(symbol, market);
        }
        cache
    }

    pub fn into_exchange_info(&self) -> ExchangeInfo {
        let symbols = self
            .order_books
            .iter()
            .filter(|m| m.market_type == "perp")
            .map(|meta| {
                let symbol = lighter_symbol_to_unified(&meta.symbol);
                let status = match meta.status.as_str() {
                    "active" => SymbolStatus::Trading,
                    "halted" => SymbolStatus::Halted,
                    _ => SymbolStatus::Unknown,
                };
                let tick_size = tick_size_from_decimals(meta.supported_price_decimals);

                SymbolInfo {
                    symbol,
                    raw_symbol: meta.symbol.clone(),
                    status,
                    base_precision: meta.supported_size_decimals,
                    quote_precision: meta.supported_price_decimals,
                    min_qty: Decimal::from_str(&meta.min_base_amount).ok(),
                    min_notional: Decimal::from_str(&meta.min_quote_amount).ok(),
                    tick_size: Some(tick_size),
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::LighterFutures,
            symbols,
        }
    }
}

fn tick_size_from_decimals(decimals: u8) -> Decimal {
    if decimals == 0 {
        return Decimal::ONE;
    }
    Decimal::ONE / Decimal::from(10u64.pow(decimals as u32))
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/orderBookDetails  (ticker-like details)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterOrderBookDetailsResponse {
    pub code: u16,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub order_book_details: Vec<LighterOrderBookDetail>,
}

#[derive(Debug, Deserialize)]
pub struct LighterOrderBookDetail {
    pub symbol: String,
    pub market_id: u16,
    #[serde(default)]
    pub market_type: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub last_trade_price: f64,
    #[serde(default)]
    pub daily_trades_count: u64,
    #[serde(default)]
    pub daily_base_token_volume: f64,
    #[serde(default)]
    pub daily_quote_token_volume: f64,
    #[serde(default)]
    pub daily_price_low: f64,
    #[serde(default)]
    pub daily_price_high: f64,
    #[serde(default)]
    pub daily_price_change: f64,
    #[serde(default)]
    pub open_interest: f64,
}

impl LighterOrderBookDetail {
    pub fn into_ticker(self) -> Ticker {
        let symbol = lighter_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::LighterFutures,
            symbol,
            last_price: Decimal::from_str(&self.last_trade_price.to_string())
                .unwrap_or_default(),
            bid: None,
            ask: None,
            volume_24h: Decimal::from_str(&self.daily_base_token_volume.to_string())
                .unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.daily_price_change.to_string()).ok(),
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/orderBookOrders  (order book depth)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterOrderBookOrdersResponse {
    pub code: u16,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub asks: Vec<LighterRestOrderLevel>,
    #[serde(default)]
    pub bids: Vec<LighterRestOrderLevel>,
}

/// REST order book level – uses `remaining_base_amount` as the size field.
#[derive(Debug, Deserialize)]
pub struct LighterRestOrderLevel {
    pub price: String,
    pub remaining_base_amount: String,
}

impl LighterOrderBookOrdersResponse {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::LighterFutures,
            symbol,
            bids: parse_rest_levels(&self.bids),
            asks: parse_rest_levels(&self.asks),
            timestamp_ms: 0,
            sequence: None,
        }
    }
}

/// WS order book level – uses `size` as the size field.
#[derive(Debug, Deserialize)]
pub struct LighterWsOrderBookLevel {
    pub price: String,
    pub size: String,
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/recentTrades
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterRecentTradesResponse {
    pub code: u16,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub trades: Vec<LighterTradeRaw>,
}

#[derive(Debug, Deserialize)]
pub struct LighterTradeRaw {
    #[serde(default)]
    pub trade_id: u64,
    pub market_id: u16,
    pub size: String,
    pub price: String,
    #[serde(default)]
    pub is_maker_ask: bool,
    #[serde(default)]
    pub timestamp: u64,
}

impl LighterTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let side = if self.is_maker_ask {
            Side::Buy
        } else {
            Side::Sell
        };
        Trade {
            exchange: ExchangeId::LighterFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.timestamp * 1000,
            trade_id: Some(self.trade_id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/candles
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterCandlesResponse {
    pub code: u16,
    #[serde(default)]
    pub message: String,
    /// Resolution string (e.g. "1h").
    #[serde(default)]
    pub r: String,
    /// Candlestick array.
    #[serde(default)]
    pub c: Vec<LighterCandleRaw>,
}

/// Single candlestick from the Lighter `/api/v1/candles` endpoint.
/// Fields are abbreviated: t=timestamp, o=open, h=high, l=low, c=close, v=volume.
#[derive(Debug, Deserialize)]
pub struct LighterCandleRaw {
    #[serde(default)]
    pub t: u64,
    #[serde(default)]
    pub o: f64,
    #[serde(default)]
    pub h: f64,
    #[serde(default)]
    pub l: f64,
    #[serde(default)]
    pub c: f64,
    #[serde(default)]
    pub v: f64,
}

impl LighterCandleRaw {
    pub fn into_candle(self, symbol: Symbol, interval: Interval) -> Candle {
        Candle {
            exchange: ExchangeId::LighterFutures,
            symbol,
            open: f64_to_decimal(self.o),
            high: f64_to_decimal(self.h),
            low: f64_to_decimal(self.l),
            close: f64_to_decimal(self.c),
            volume: f64_to_decimal(self.v),
            open_time_ms: self.t,
            close_time_ms: self.t + interval.as_secs() * 1000,
            is_closed: true,
        }
    }
}

// ---------------------------------------------------------------------------
// REST: GET /api/v1/funding-rates
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterFundingRatesResponse {
    pub code: u16,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub funding_rates: Vec<LighterFundingRateRaw>,
}

#[derive(Debug, Deserialize)]
pub struct LighterFundingRateRaw {
    pub market_id: u16,
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub symbol: String,
    pub rate: f64,
}

// ---------------------------------------------------------------------------
// WebSocket: market_stats channel (mark price, funding, OI)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterWsMarketStats {
    pub market_id: u16,
    #[serde(default)]
    pub index_price: String,
    #[serde(default)]
    pub mark_price: String,
    #[serde(default)]
    pub open_interest: String,
    #[serde(default)]
    pub last_trade_price: String,
    #[serde(default)]
    pub current_funding_rate: String,
    #[serde(default)]
    pub funding_rate: String,
    #[serde(default)]
    pub funding_timestamp: u64,
    #[serde(default)]
    pub daily_base_token_volume: f64,
    #[serde(default)]
    pub daily_quote_token_volume: f64,
    #[serde(default)]
    pub daily_price_low: f64,
    #[serde(default)]
    pub daily_price_high: f64,
    #[serde(default)]
    pub daily_price_change: f64,
}

impl LighterWsMarketStats {
    pub fn into_mark_price(self, symbol: Symbol) -> MarkPrice {
        MarkPrice {
            exchange: ExchangeId::LighterFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.funding_timestamp,
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket: order_book channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterWsOrderBookUpdate {
    #[serde(default)]
    pub code: u16,
    #[serde(default)]
    pub asks: Vec<LighterWsOrderBookLevel>,
    #[serde(default)]
    pub bids: Vec<LighterWsOrderBookLevel>,
    #[serde(default)]
    pub offset: u64,
}

impl LighterWsOrderBookUpdate {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::LighterFutures,
            symbol,
            bids: parse_ws_levels(&self.bids),
            asks: parse_ws_levels(&self.asks),
            timestamp_ms: 0,
            sequence: Some(self.offset),
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket: trade channel
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LighterWsTrade {
    #[serde(default)]
    pub trade_id: u64,
    pub market_id: u16,
    pub size: String,
    pub price: String,
    #[serde(default)]
    pub is_maker_ask: bool,
    #[serde(default)]
    pub timestamp: u64,
}

impl LighterWsTrade {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let side = if self.is_maker_ask {
            Side::Buy
        } else {
            Side::Sell
        };
        Trade {
            exchange: ExchangeId::LighterFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.timestamp * 1000,
            trade_id: Some(self.trade_id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol conversion helpers
// ---------------------------------------------------------------------------

/// Convert Lighter raw symbol (e.g. "ETH") to unified Symbol (ETH/USDC).
/// Lighter perp markets are all quoted in USDC.
pub fn lighter_symbol_to_unified(raw: &str) -> Symbol {
    Symbol::new(raw, "USDC")
}

/// Convert Interval to Lighter resolution string.
pub fn interval_to_lighter(interval: Interval) -> &'static str {
    match interval {
        Interval::M1 => "1m",
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "1w",
        // Lighter doesn't support these; map to closest.
        Interval::S1 => "1m",
        Interval::M3 => "5m",
    }
}

/// Extract market_id from a WS channel string like "trade:0" or "order_book:0".
pub fn parse_market_id_from_channel(channel: &str) -> Option<u16> {
    channel.rsplit(':').next()?.parse().ok()
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn f64_to_decimal(v: f64) -> Decimal {
    Decimal::from_str(&v.to_string()).unwrap_or_default()
}

fn parse_ws_levels(raw: &[LighterWsOrderBookLevel]) -> Vec<Level> {
    raw.iter()
        .filter_map(|lvl| {
            let price = Decimal::from_str(&lvl.price).ok()?;
            let qty = Decimal::from_str(&lvl.size).ok()?;
            Some(Level::new(price, qty))
        })
        .collect()
}

fn parse_rest_levels(raw: &[LighterRestOrderLevel]) -> Vec<Level> {
    raw.iter()
        .filter_map(|lvl| {
            let price = Decimal::from_str(&lvl.price).ok()?;
            let qty = Decimal::from_str(&lvl.remaining_base_amount).ok()?;
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
    fn test_lighter_symbol_to_unified() {
        let sym = lighter_symbol_to_unified("ETH");
        assert_eq!(sym, Symbol::new("ETH", "USDC"));
    }

    #[test]
    fn test_lighter_symbol_btc() {
        let sym = lighter_symbol_to_unified("BTC");
        assert_eq!(sym, Symbol::new("BTC", "USDC"));
    }

    #[test]
    fn test_interval_to_lighter() {
        assert_eq!(interval_to_lighter(Interval::M1), "1m");
        assert_eq!(interval_to_lighter(Interval::M5), "5m");
        assert_eq!(interval_to_lighter(Interval::M30), "30m");
        assert_eq!(interval_to_lighter(Interval::H1), "1h");
        assert_eq!(interval_to_lighter(Interval::D1), "1d");
        assert_eq!(interval_to_lighter(Interval::W1), "1w");
    }

    #[test]
    fn test_parse_market_id_from_channel() {
        assert_eq!(parse_market_id_from_channel("trade:0"), Some(0));
        assert_eq!(parse_market_id_from_channel("order_book:5"), Some(5));
        assert_eq!(parse_market_id_from_channel("market_stats:12"), Some(12));
        assert_eq!(parse_market_id_from_channel("invalid"), None);
    }

    #[test]
    fn test_tick_size_from_decimals() {
        assert_eq!(tick_size_from_decimals(0), Decimal::ONE);
        assert_eq!(tick_size_from_decimals(2), dec!(0.01));
        assert_eq!(tick_size_from_decimals(4), dec!(0.0001));
    }

    #[test]
    fn test_parse_ws_levels() {
        let raw = vec![
            LighterWsOrderBookLevel {
                price: "3335.04".to_string(),
                size: "10.5".to_string(),
            },
            LighterWsOrderBookLevel {
                price: "3336.00".to_string(),
                size: "5.0".to_string(),
            },
        ];
        let levels = parse_ws_levels(&raw);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(3335.04));
        assert_eq!(levels[0].qty, dec!(10.5));
    }

    #[test]
    fn test_parse_rest_levels() {
        let raw = vec![
            LighterRestOrderLevel {
                price: "1916.92".to_string(),
                remaining_base_amount: "0.9440".to_string(),
            },
        ];
        let levels = parse_rest_levels(&raw);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, dec!(1916.92));
        assert_eq!(levels[0].qty, dec!(0.9440));
    }

    #[test]
    fn test_order_books_response_into_exchange_info() {
        let resp: LighterOrderBooksResponse = serde_json::from_str(
            r#"{
                "code": 200,
                "message": "",
                "order_books": [
                    {
                        "symbol": "ETH",
                        "market_id": 0,
                        "market_type": "perp",
                        "status": "active",
                        "taker_fee": "0.0001",
                        "maker_fee": "0.0000",
                        "min_base_amount": "0.01",
                        "min_quote_amount": "0.1",
                        "supported_size_decimals": 4,
                        "supported_price_decimals": 4
                    },
                    {
                        "symbol": "USDC",
                        "market_id": 10,
                        "market_type": "spot",
                        "status": "active",
                        "taker_fee": "0.001",
                        "maker_fee": "0.000",
                        "min_base_amount": "1",
                        "min_quote_amount": "1",
                        "supported_size_decimals": 2,
                        "supported_price_decimals": 2
                    }
                ]
            }"#,
        )
        .unwrap();

        let info = resp.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::LighterFutures);
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(info.symbols[0].raw_symbol, "ETH");
        assert_eq!(info.symbols[0].status, SymbolStatus::Trading);
        assert_eq!(info.symbols[0].base_precision, 4);
        assert_eq!(info.symbols[0].quote_precision, 4);
        assert_eq!(info.symbols[0].min_qty, Some(dec!(0.01)));
    }

    #[test]
    fn test_order_books_response_into_market_cache() {
        let resp: LighterOrderBooksResponse = serde_json::from_str(
            r#"{
                "code": 200,
                "message": "",
                "order_books": [
                    {
                        "symbol": "ETH",
                        "market_id": 0,
                        "market_type": "perp",
                        "status": "active",
                        "taker_fee": "0.0001",
                        "maker_fee": "0.0000",
                        "min_base_amount": "0.01",
                        "min_quote_amount": "0.1",
                        "supported_size_decimals": 4,
                        "supported_price_decimals": 4
                    },
                    {
                        "symbol": "BTC",
                        "market_id": 1,
                        "market_type": "perp",
                        "status": "active",
                        "taker_fee": "0.0001",
                        "maker_fee": "0.0000",
                        "min_base_amount": "0.001",
                        "min_quote_amount": "1",
                        "supported_size_decimals": 5,
                        "supported_price_decimals": 2
                    }
                ]
            }"#,
        )
        .unwrap();

        let cache = resp.into_market_cache();
        assert_eq!(cache.market_id(&Symbol::new("ETH", "USDC")), Some(0));
        assert_eq!(cache.market_id(&Symbol::new("BTC", "USDC")), Some(1));
        assert_eq!(cache.symbol(0), Some(Symbol::new("ETH", "USDC")));
        assert_eq!(cache.symbol(1), Some(Symbol::new("BTC", "USDC")));
    }

    #[test]
    fn test_trade_raw_conversion() {
        let raw = LighterTradeRaw {
            trade_id: 14035051,
            market_id: 0,
            size: "0.1187".to_string(),
            price: "3335.65".to_string(),
            is_maker_ask: false,
            timestamp: 1722339648,
        };
        let trade = raw.into_trade(Symbol::new("ETH", "USDC"));
        assert_eq!(trade.exchange, ExchangeId::LighterFutures);
        assert_eq!(trade.symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(trade.price, dec!(3335.65));
        assert_eq!(trade.qty, dec!(0.1187));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.timestamp_ms, 1722339648000);
        assert_eq!(trade.trade_id, Some("14035051".to_string()));
    }

    #[test]
    fn test_trade_raw_buy_side() {
        let raw = LighterTradeRaw {
            trade_id: 100,
            market_id: 0,
            size: "1.0".to_string(),
            price: "3000.00".to_string(),
            is_maker_ask: true,
            timestamp: 1700000000,
        };
        let trade = raw.into_trade(Symbol::new("ETH", "USDC"));
        assert_eq!(trade.side, Side::Buy);
    }

    #[test]
    fn test_ws_orderbook_update_conversion() {
        let raw: LighterWsOrderBookUpdate = serde_json::from_str(
            r#"{
                "code": 0,
                "asks": [{"price": "3327.46", "size": "29.0915"}],
                "bids": [{"price": "3338.80", "size": "10.2898"}],
                "offset": 41692864
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("ETH", "USDC"));
        assert_eq!(ob.exchange, ExchangeId::LighterFutures);
        assert_eq!(ob.symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks[0].price, dec!(3327.46));
        assert_eq!(ob.bids[0].price, dec!(3338.80));
        assert_eq!(ob.sequence, Some(41692864));
    }

    #[test]
    fn test_ws_market_stats_into_mark_price() {
        let raw: LighterWsMarketStats = serde_json::from_str(
            r#"{
                "market_id": 0,
                "index_price": "3335.04",
                "mark_price": "3335.09",
                "open_interest": "235.25",
                "last_trade_price": "3335.65",
                "current_funding_rate": "0.0057",
                "funding_rate": "0.0005",
                "funding_timestamp": 1722337200000,
                "daily_base_token_volume": 230206.49,
                "daily_quote_token_volume": 765295250.98,
                "daily_price_low": 3265.13,
                "daily_price_high": 3386.01,
                "daily_price_change": -1.156
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price(Symbol::new("ETH", "USDC"));
        assert_eq!(mp.exchange, ExchangeId::LighterFutures);
        assert_eq!(mp.symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(mp.mark_price, dec!(3335.09));
        assert_eq!(mp.index_price, dec!(3335.04));
        assert_eq!(mp.timestamp_ms, 1722337200000);
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw: LighterWsTrade = serde_json::from_str(
            r#"{
                "trade_id": 14035051,
                "market_id": 0,
                "size": "0.1187",
                "price": "3335.65",
                "is_maker_ask": false,
                "timestamp": 1722339648
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade(Symbol::new("ETH", "USDC"));
        assert_eq!(trade.price, dec!(3335.65));
        assert_eq!(trade.qty, dec!(0.1187));
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_order_book_detail_into_ticker() {
        let raw = LighterOrderBookDetail {
            symbol: "ETH".to_string(),
            market_id: 0,
            market_type: "perp".to_string(),
            status: "active".to_string(),
            last_trade_price: 3024.66,
            daily_trades_count: 68,
            daily_base_token_volume: 1234.5,
            daily_quote_token_volume: 9999.0,
            daily_price_low: 3000.0,
            daily_price_high: 3100.0,
            daily_price_change: 3.66,
            open_interest: 500.0,
        };
        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::LighterFutures);
        assert_eq!(ticker.symbol, Symbol::new("ETH", "USDC"));
        assert_eq!(ticker.last_price, dec!(3024.66));
        assert_eq!(ticker.volume_24h, dec!(1234.5));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(3.66)));
    }

    #[test]
    fn test_orderbook_orders_response_conversion() {
        let resp: LighterOrderBookOrdersResponse = serde_json::from_str(
            r#"{
                "code": 200,
                "message": "",
                "asks": [{"price": "1916.92", "remaining_base_amount": "0.9440", "order_id": "1"}],
                "bids": [{"price": "1916.74", "remaining_base_amount": "0.5310", "order_id": "2"}]
            }"#,
        )
        .unwrap();

        let ob = resp.into_orderbook(Symbol::new("ETH", "USDC"));
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks[0].price, dec!(1916.92));
        assert_eq!(ob.asks[0].qty, dec!(0.9440));
        assert_eq!(ob.bids[0].price, dec!(1916.74));
        assert_eq!(ob.bids[0].qty, dec!(0.5310));
    }

    #[test]
    fn test_candle_raw_conversion() {
        let raw = LighterCandleRaw {
            t: 1772211600000,
            o: 1925.95,
            h: 1933.48,
            l: 1906.18,
            c: 1930.83,
            v: 14664.3905,
        };
        let candle = raw.into_candle(Symbol::new("ETH", "USDC"), Interval::H1);
        assert_eq!(candle.exchange, ExchangeId::LighterFutures);
        assert_eq!(candle.open, dec!(1925.95));
        assert_eq!(candle.high, dec!(1933.48));
        assert_eq!(candle.low, dec!(1906.18));
        assert_eq!(candle.close, dec!(1930.83));
        assert_eq!(candle.open_time_ms, 1772211600000);
        assert_eq!(candle.close_time_ms, 1772211600000 + 3600 * 1000);
    }

    #[test]
    fn test_f64_to_decimal() {
        assert_eq!(f64_to_decimal(1925.95), dec!(1925.95));
        assert_eq!(f64_to_decimal(0.0), Decimal::ZERO);
    }
}
