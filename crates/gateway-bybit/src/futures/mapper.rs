use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// Re-use spot helpers for symbol/interval conversion.
pub use crate::spot::mapper::{
    bybit_symbol_to_unified, bybit_status_to_unified, interval_to_bybit, unified_to_bybit,
    BybitResponse,
};

// ---------------------------------------------------------------------------
// Instruments Info (GET /v5/market/instruments-info?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitLinearInstrumentsResult {
    pub category: String,
    pub list: Vec<BybitLinearInstrumentRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearInstrumentRaw {
    pub symbol: String,
    #[serde(rename = "baseCoin")]
    pub base_coin: String,
    #[serde(rename = "quoteCoin")]
    pub quote_coin: String,
    pub status: String,
    #[serde(rename = "lotSizeFilter")]
    pub lot_size_filter: BybitLinearLotSizeFilter,
    #[serde(rename = "priceFilter")]
    pub price_filter: BybitLinearPriceFilter,
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearLotSizeFilter {
    #[serde(rename = "minOrderQty")]
    pub min_order_qty: String,
    #[serde(rename = "qtyStep")]
    pub qty_step: String,
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearPriceFilter {
    #[serde(rename = "tickSize")]
    pub tick_size: String,
}

impl BybitLinearInstrumentsResult {
    pub fn into_exchange_info(self) -> ExchangeInfo {
        let symbols = self
            .list
            .into_iter()
            .map(|raw| {
                let status = bybit_status_to_unified(&raw.status);
                let base_precision = decimal_precision(&raw.lot_size_filter.qty_step);
                let quote_precision = decimal_precision(&raw.price_filter.tick_size);
                let min_qty = Decimal::from_str(&raw.lot_size_filter.min_order_qty).ok();
                let tick_size = Decimal::from_str(&raw.price_filter.tick_size).ok();

                SymbolInfo {
                    symbol: Symbol::new(&raw.base_coin, &raw.quote_coin),
                    raw_symbol: raw.symbol,
                    status,
                    base_precision,
                    quote_precision,
                    min_qty,
                    min_notional: None,
                    tick_size,
                }
            })
            .collect();

        ExchangeInfo {
            exchange: ExchangeId::BybitFutures,
            symbols,
        }
    }
}

/// Compute the number of decimal places from a precision string like "0.001".
fn decimal_precision(s: &str) -> u8 {
    match s.find('.') {
        Some(pos) => {
            let decimals = &s[pos + 1..];
            decimals.len() as u8
        }
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// REST OrderBook (GET /v5/market/orderbook?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitLinearOrderBookResult {
    pub s: String,
    pub b: Vec<[String; 2]>,
    pub a: Vec<[String; 2]>,
    pub u: u64,
    pub ts: u64,
}

impl BybitLinearOrderBookResult {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = bybit_symbol_to_unified(&self.s);
        OrderBook {
            exchange: ExchangeId::BybitFutures,
            symbol,
            bids: parse_levels(&self.b),
            asks: parse_levels(&self.a),
            timestamp_ms: self.ts,
            sequence: Some(self.u),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Trades (GET /v5/market/recent-trade?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitLinearTradesResult {
    pub category: String,
    pub list: Vec<BybitLinearTradeRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearTradeRaw {
    #[serde(rename = "execId")]
    pub exec_id: String,
    pub symbol: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub time: String,
}

impl BybitLinearTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::BybitFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.time.parse::<u64>().unwrap_or(0),
            trade_id: Some(self.exec_id),
        }
    }
}

// ---------------------------------------------------------------------------
// REST Tickers (GET /v5/market/tickers?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitLinearTickersResult {
    pub category: String,
    pub list: Vec<BybitLinearTickerRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "bid1Price")]
    pub bid1_price: String,
    #[serde(rename = "ask1Price")]
    pub ask1_price: String,
    #[serde(rename = "volume24h")]
    pub volume_24h: String,
    #[serde(rename = "price24hPcnt")]
    pub price_24h_pcnt: String,
    #[serde(rename = "highPrice24h")]
    pub high_price_24h: String,
    #[serde(rename = "lowPrice24h")]
    pub low_price_24h: String,
    #[serde(rename = "turnover24h")]
    pub turnover_24h: String,
    // Futures-specific fields:
    #[serde(rename = "fundingRate")]
    pub funding_rate: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
    #[serde(rename = "indexPrice")]
    pub index_price: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
}

impl BybitLinearTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::BybitFutures,
            symbol,
            last_price: Decimal::from_str(&self.last_price).unwrap_or_default(),
            bid: Decimal::from_str(&self.bid1_price).ok(),
            ask: Decimal::from_str(&self.ask1_price).ok(),
            volume_24h: Decimal::from_str(&self.volume_24h).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.price_24h_pcnt).ok(),
            timestamp_ms: 0,
        }
    }

    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::BybitFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// REST Klines (GET /v5/market/kline?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitLinearKlinesResult {
    pub category: String,
    pub symbol: String,
    pub list: Vec<Vec<String>>,
}

/// Parse a single Bybit linear kline row into a Candle.
///
/// Row format: [startTime, open, high, low, close, volume, turnover]
pub fn parse_kline_row(row: &[String], symbol: Symbol) -> Option<Candle> {
    if row.len() < 7 {
        return None;
    }
    let open_time_ms = row[0].parse::<u64>().ok()?;
    let open = Decimal::from_str(&row[1]).ok()?;
    let high = Decimal::from_str(&row[2]).ok()?;
    let low = Decimal::from_str(&row[3]).ok()?;
    let close = Decimal::from_str(&row[4]).ok()?;
    let volume = Decimal::from_str(&row[5]).ok()?;

    Some(Candle {
        exchange: ExchangeId::BybitFutures,
        symbol,
        open,
        high,
        low,
        close,
        volume,
        open_time_ms,
        close_time_ms: 0,
        is_closed: true,
    })
}

// ---------------------------------------------------------------------------
// Funding Rate (GET /v5/market/funding/history?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitFundingHistoryResult {
    pub category: String,
    pub list: Vec<BybitFundingHistoryRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitFundingHistoryRaw {
    pub symbol: String,
    #[serde(rename = "fundingRate")]
    pub funding_rate: String,
    #[serde(rename = "fundingRateTimestamp")]
    pub funding_rate_timestamp: String,
}

impl BybitFundingHistoryRaw {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        FundingRate {
            exchange: ExchangeId::BybitFutures,
            symbol,
            rate: Decimal::from_str(&self.funding_rate).unwrap_or_default(),
            // Bybit returns the timestamp of the funding event; next funding
            // time is not directly available from this endpoint.
            next_funding_time_ms: 0,
            timestamp_ms: self.funding_rate_timestamp.parse::<u64>().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Open Interest (GET /v5/market/open-interest?category=linear)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitOpenInterestResult {
    pub category: String,
    pub list: Vec<BybitOpenInterestRaw>,
}

#[derive(Debug, Deserialize)]
pub struct BybitOpenInterestRaw {
    #[serde(rename = "openInterest")]
    pub open_interest: String,
    pub timestamp: String,
}

impl BybitOpenInterestRaw {
    pub fn into_open_interest(self, symbol: Symbol) -> OpenInterest {
        let oi = Decimal::from_str(&self.open_interest).unwrap_or_default();
        OpenInterest {
            exchange: ExchangeId::BybitFutures,
            symbol,
            open_interest: oi,
            // Bybit open-interest endpoint returns quantity only, not notional value.
            open_interest_value: Decimal::ZERO,
            timestamp_ms: self.timestamp.parse::<u64>().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BybitWsMessage<T> {
    pub topic: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub ts: u64,
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearWsOrderBook {
    pub s: String,
    pub b: Vec<[String; 2]>,
    pub a: Vec<[String; 2]>,
    pub u: u64,
    pub seq: u64,
}

impl BybitLinearWsOrderBook {
    pub fn into_orderbook(self) -> OrderBook {
        let symbol = bybit_symbol_to_unified(&self.s);
        OrderBook {
            exchange: ExchangeId::BybitFutures,
            symbol,
            bids: parse_levels(&self.b),
            asks: parse_levels(&self.a),
            timestamp_ms: 0,
            sequence: Some(self.u),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearWsTrade {
    #[serde(rename = "T")]
    pub trade_time: u64,
    pub s: String,
    #[serde(rename = "S")]
    pub side: String,
    pub v: String,
    pub p: String,
    pub i: String,
}

impl BybitLinearWsTrade {
    pub fn into_trade(self) -> Trade {
        let symbol = bybit_symbol_to_unified(&self.s);
        let side = match self.side.as_str() {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::BybitFutures,
            symbol,
            price: Decimal::from_str(&self.p).unwrap_or_default(),
            qty: Decimal::from_str(&self.v).unwrap_or_default(),
            side,
            timestamp_ms: self.trade_time,
            trade_id: Some(self.i),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BybitLinearWsKlineData {
    pub start: u64,
    pub end: u64,
    pub interval: String,
    pub open: String,
    pub close: String,
    pub high: String,
    pub low: String,
    pub volume: String,
    pub confirm: bool,
}

impl BybitLinearWsKlineData {
    pub fn into_candle(self, symbol: Symbol) -> Candle {
        Candle {
            exchange: ExchangeId::BybitFutures,
            symbol,
            open: Decimal::from_str(&self.open).unwrap_or_default(),
            high: Decimal::from_str(&self.high).unwrap_or_default(),
            low: Decimal::from_str(&self.low).unwrap_or_default(),
            close: Decimal::from_str(&self.close).unwrap_or_default(),
            volume: Decimal::from_str(&self.volume).unwrap_or_default(),
            open_time_ms: self.start,
            close_time_ms: self.end,
            is_closed: self.confirm,
        }
    }
}

/// WS linear ticker — includes futures-specific fields.
#[derive(Debug, Deserialize)]
pub struct BybitLinearWsTicker {
    pub symbol: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
    #[serde(rename = "indexPrice")]
    pub index_price: String,
}

impl BybitLinearWsTicker {
    pub fn into_mark_price(self, ts: u64) -> MarkPrice {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::BybitFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: ts,
        }
    }
}

/// WS liquidation event (topic: `liquidation.{SYMBOL}`).
#[derive(Debug, Deserialize)]
pub struct BybitWsLiquidation {
    pub symbol: String,
    pub side: String,
    pub price: String,
    pub size: String,
    #[serde(rename = "updatedTime")]
    pub updated_time: String,
}

impl BybitWsLiquidation {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = bybit_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Liquidation {
            exchange: ExchangeId::BybitFutures,
            symbol,
            side,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            timestamp_ms: self.updated_time.parse::<u64>().unwrap_or(0),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BybitWsResponse {
    pub op: String,
    pub success: Option<bool>,
    #[serde(rename = "conn_id")]
    pub conn_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper functions
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
    fn test_linear_orderbook_conversion() {
        let raw: BybitLinearOrderBookResult = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "b": [["50000.00", "1.0"], ["49999.00", "0.5"]],
                "a": [["50001.00", "2.0"]],
                "u": 42,
                "ts": 1700000000000
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::BybitFutures);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.bids[1].price, dec!(49999.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert_eq!(ob.sequence, Some(42));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_linear_trade_conversion() {
        let raw: BybitLinearTradeRaw = serde_json::from_str(
            r#"{
                "execId": "abc123",
                "symbol": "ETHUSDT",
                "price": "2000.50",
                "size": "0.5",
                "side": "Buy",
                "time": "1700000000000"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::BybitFutures);
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.50));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("abc123".to_string()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_linear_trade_sell_side() {
        let raw = BybitLinearTradeRaw {
            exec_id: "def456".to_string(),
            symbol: "BTCUSDT".to_string(),
            price: "50000.00".to_string(),
            size: "0.01".to_string(),
            side: "Sell".to_string(),
            time: "1700000000001".to_string(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_linear_ticker_conversion() {
        let raw: BybitLinearTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPrice": "50000.00",
                "bid1Price": "49999.00",
                "ask1Price": "50001.00",
                "volume24h": "12345.678",
                "price24hPcnt": "0.025",
                "highPrice24h": "51000.00",
                "lowPrice24h": "49000.00",
                "turnover24h": "617283900.00",
                "fundingRate": "0.0001",
                "markPrice": "50010.00",
                "indexPrice": "50005.00",
                "openInterest": "9876.54"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::BybitFutures);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(0.025)));
    }

    #[test]
    fn test_linear_ticker_into_mark_price() {
        let raw: BybitLinearTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "ETHUSDT",
                "lastPrice": "2000.00",
                "bid1Price": "1999.00",
                "ask1Price": "2001.00",
                "volume24h": "5000.0",
                "price24hPcnt": "0.01",
                "highPrice24h": "2100.00",
                "lowPrice24h": "1900.00",
                "turnover24h": "10000000.00",
                "fundingRate": "0.0002",
                "markPrice": "2000.50",
                "indexPrice": "2000.00",
                "openInterest": "1234.56"
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::BybitFutures);
        assert_eq!(mp.symbol, Symbol::new("ETH", "USDT"));
        assert_eq!(mp.mark_price, dec!(2000.50));
        assert_eq!(mp.index_price, dec!(2000.00));
    }

    #[test]
    fn test_parse_kline_row_linear() {
        let row = vec![
            "1700000000000".to_string(),
            "50000.00".to_string(),
            "50200.00".to_string(),
            "49900.00".to_string(),
            "50100.00".to_string(),
            "100.5".to_string(),
            "5025000.00".to_string(),
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::BybitFutures);
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert!(candle.is_closed);
    }

    #[test]
    fn test_parse_kline_row_too_short() {
        let row = vec!["1700000000000".to_string(), "50000.00".to_string()];
        assert!(parse_kline_row(&row, Symbol::new("BTC", "USDT")).is_none());
    }

    #[test]
    fn test_funding_rate_conversion() {
        let raw = BybitFundingHistoryRaw {
            symbol: "BTCUSDT".to_string(),
            funding_rate: "0.0001".to_string(),
            funding_rate_timestamp: "1700000000000".to_string(),
        };
        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::BybitFutures);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(fr.rate, dec!(0.0001));
        assert_eq!(fr.timestamp_ms, 1700000000000);
        assert_eq!(fr.next_funding_time_ms, 0);
    }

    #[test]
    fn test_open_interest_conversion() {
        let raw = BybitOpenInterestRaw {
            open_interest: "12345.678".to_string(),
            timestamp: "1700000000000".to_string(),
        };
        let oi = raw.into_open_interest(Symbol::new("BTC", "USDT"));
        assert_eq!(oi.exchange, ExchangeId::BybitFutures);
        assert_eq!(oi.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(oi.open_interest, dec!(12345.678));
        assert_eq!(oi.open_interest_value, Decimal::ZERO);
        assert_eq!(oi.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_liquidation_conversion() {
        let raw: BybitWsLiquidation = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "side": "Sell",
                "price": "48000.00",
                "size": "1.5",
                "updatedTime": "1700000000100"
            }"#,
        )
        .unwrap();

        let liq = raw.into_liquidation();
        assert_eq!(liq.exchange, ExchangeId::BybitFutures);
        assert_eq!(liq.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(liq.side, Side::Sell);
        assert_eq!(liq.price, dec!(48000.00));
        assert_eq!(liq.qty, dec!(1.5));
        assert_eq!(liq.timestamp_ms, 1700000000100);
    }

    #[test]
    fn test_ws_liquidation_buy_side() {
        let raw = BybitWsLiquidation {
            symbol: "ETHUSDT".to_string(),
            side: "Buy".to_string(),
            price: "2100.00".to_string(),
            size: "5.0".to_string(),
            updated_time: "1700000000200".to_string(),
        };
        let liq = raw.into_liquidation();
        assert_eq!(liq.side, Side::Buy);
        assert_eq!(liq.symbol, Symbol::new("ETH", "USDT"));
    }

    #[test]
    fn test_ws_linear_orderbook_conversion() {
        let raw: BybitLinearWsOrderBook = serde_json::from_str(
            r#"{
                "s": "BTCUSDT",
                "b": [["50000.00", "1.0"]],
                "a": [["50001.00", "2.0"]],
                "u": 100,
                "seq": 200
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook();
        assert_eq!(ob.exchange, ExchangeId::BybitFutures);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(100));
    }

    #[test]
    fn test_ws_linear_trade_conversion() {
        let raw: BybitLinearWsTrade = serde_json::from_str(
            r#"{
                "T": 1700000000000,
                "s": "ETHUSDT",
                "S": "Sell",
                "v": "0.5",
                "p": "2000.00",
                "i": "trade-999"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::BybitFutures);
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.00));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.trade_id, Some("trade-999".to_string()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_linear_kline_conversion() {
        let raw: BybitLinearWsKlineData = serde_json::from_str(
            r#"{
                "start": 1700000000000,
                "end": 1700000060000,
                "interval": "1",
                "open": "50000.00",
                "close": "50100.00",
                "high": "50200.00",
                "low": "49900.00",
                "volume": "100.5",
                "confirm": true
            }"#,
        )
        .unwrap();

        let candle = raw.into_candle(Symbol::new("BTC", "USDT"));
        assert_eq!(candle.exchange, ExchangeId::BybitFutures);
        assert_eq!(candle.symbol.base, "BTC");
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.volume, dec!(100.5));
        assert_eq!(candle.open_time_ms, 1700000000000);
        assert_eq!(candle.close_time_ms, 1700000060000);
        assert!(candle.is_closed);
    }

    #[test]
    fn test_ws_linear_kline_not_confirmed() {
        let raw = BybitLinearWsKlineData {
            start: 1700000000000,
            end: 1700000060000,
            interval: "1".to_string(),
            open: "50000.00".to_string(),
            close: "50050.00".to_string(),
            high: "50100.00".to_string(),
            low: "49950.00".to_string(),
            volume: "10.0".to_string(),
            confirm: false,
        };
        let candle = raw.into_candle(Symbol::new("BTC", "USDT"));
        assert!(!candle.is_closed);
    }

    #[test]
    fn test_ws_mark_price_from_ticker() {
        let raw: BybitLinearWsTicker = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "markPrice": "50123.45",
                "indexPrice": "50100.00"
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price(1700000000000);
        assert_eq!(mp.exchange, ExchangeId::BybitFutures);
        assert_eq!(mp.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(mp.mark_price, dec!(50123.45));
        assert_eq!(mp.index_price, dec!(50100.00));
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_exchange_info_conversion() {
        let raw: BybitResponse<BybitLinearInstrumentsResult> = serde_json::from_str(
            r#"{
                "retCode": 0,
                "retMsg": "OK",
                "result": {
                    "category": "linear",
                    "list": [{
                        "symbol": "BTCUSDT",
                        "baseCoin": "BTC",
                        "quoteCoin": "USDT",
                        "status": "Trading",
                        "lotSizeFilter": {
                            "minOrderQty": "0.001",
                            "qtyStep": "0.001"
                        },
                        "priceFilter": {
                            "tickSize": "0.10"
                        }
                    }, {
                        "symbol": "ETHUSDT",
                        "baseCoin": "ETH",
                        "quoteCoin": "USDT",
                        "status": "PreLaunch",
                        "lotSizeFilter": {
                            "minOrderQty": "0.01",
                            "qtyStep": "0.01"
                        },
                        "priceFilter": {
                            "tickSize": "0.01"
                        }
                    }]
                }
            }"#,
        )
        .unwrap();

        let info = raw.result.into_exchange_info();
        assert_eq!(info.exchange, ExchangeId::BybitFutures);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTCUSDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 3);
        assert_eq!(btc.quote_precision, 2);
        assert_eq!(btc.min_qty, Some(dec!(0.001)));
        assert_eq!(btc.tick_size, Some(dec!(0.10)));
        assert!(btc.min_notional.is_none());

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::PreTrading);
    }

    #[test]
    fn test_response_wrapper_deserialization() {
        let raw: BybitResponse<BybitLinearOrderBookResult> = serde_json::from_str(
            r#"{
                "retCode": 0,
                "retMsg": "OK",
                "result": {
                    "s": "BTCUSDT",
                    "b": [["50000.00", "1.0"]],
                    "a": [["50001.00", "2.0"]],
                    "u": 42,
                    "ts": 1700000000000
                }
            }"#,
        )
        .unwrap();

        assert_eq!(raw.ret_code, 0);
        assert_eq!(raw.ret_msg, "OK");
        assert_eq!(raw.result.s, "BTCUSDT");
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
        assert_eq!(levels[1].price, dec!(99.00));
        assert_eq!(levels[1].qty, dec!(2.0));
    }

    #[test]
    fn test_parse_levels_skips_invalid() {
        let raw = vec![
            ["bad".to_string(), "1.0".to_string()],
            ["50.00".to_string(), "3.0".to_string()],
        ];
        let levels = parse_levels(&raw);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, dec!(50.00));
    }

    #[test]
    fn test_decimal_precision() {
        assert_eq!(decimal_precision("0.001"), 3);
        assert_eq!(decimal_precision("0.10"), 2);
        assert_eq!(decimal_precision("0.01"), 2);
        assert_eq!(decimal_precision("1"), 0);
    }
}
