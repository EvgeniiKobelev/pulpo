use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// Re-use spot helpers
pub use crate::spot::mapper::{
    bitget_status_to_unified, bitget_symbol_to_unified, interval_to_bitget_rest,
    interval_to_bitget_ws, unified_to_bitget, BitgetResponse,
};

// ---------------------------------------------------------------------------
// Contracts (GET /api/v2/mix/market/contracts)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixContractRaw {
    pub symbol: String,
    #[serde(rename = "baseCoin")]
    pub base_coin: String,
    #[serde(rename = "quoteCoin")]
    pub quote_coin: String,
    #[serde(rename = "pricePrecision")]
    pub price_precision: String,
    #[serde(rename = "quantityPrecision")]
    pub quantity_precision: String,
    pub status: String,
    #[serde(rename = "minTradeUSDT")]
    pub min_trade_usdt: String,
}

pub fn contracts_to_exchange_info(contracts: Vec<BitgetMixContractRaw>) -> ExchangeInfo {
    let list = contracts
        .into_iter()
        .map(|raw| {
            let status = bitget_status_to_unified(&raw.status);
            let base_precision: u8 = raw.quantity_precision.parse().unwrap_or(0);
            let quote_precision: u8 = raw.price_precision.parse().unwrap_or(0);
            let min_notional = Decimal::from_str(&raw.min_trade_usdt).ok();

            SymbolInfo {
                symbol: Symbol::new(&raw.base_coin, &raw.quote_coin),
                raw_symbol: raw.symbol,
                status,
                base_precision,
                quote_precision,
                min_qty: None,
                min_notional,
                tick_size: None,
            }
        })
        .collect();

    ExchangeInfo {
        exchange: ExchangeId::BitgetFutures,
        symbols: list,
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /api/v2/mix/market/merge-depth)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixOrderBookData {
    pub asks: Vec<[String; 2]>,
    pub bids: Vec<[String; 2]>,
    pub ts: String,
}

impl BitgetMixOrderBookData {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            sequence: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Trades (GET /api/v2/mix/market/fills)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixTradeRaw {
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    pub symbol: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub ts: String,
}

impl BitgetMixTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        let side = match self.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Ticker (GET /api/v2/mix/market/ticker)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixTickerRaw {
    pub symbol: String,
    #[serde(rename = "lastPr")]
    pub last_pr: String,
    #[serde(rename = "bidPr")]
    pub bid_pr: String,
    #[serde(rename = "askPr")]
    pub ask_pr: String,
    #[serde(rename = "baseVolume")]
    pub base_volume: String,
    #[serde(rename = "change24h")]
    pub change_24h: String,
    pub ts: String,
    // Futures-specific fields
    #[serde(rename = "fundingRate")]
    pub funding_rate: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
    #[serde(rename = "indexPrice")]
    pub index_price: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
}

impl BitgetMixTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        Ticker {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            last_price: Decimal::from_str(&self.last_pr).unwrap_or_default(),
            bid: Decimal::from_str(&self.bid_pr).ok(),
            ask: Decimal::from_str(&self.ask_pr).ok(),
            volume_24h: Decimal::from_str(&self.base_volume).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.change_24h).ok(),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
        }
    }

    pub fn into_mark_price(&self) -> MarkPrice {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        MarkPrice {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Funding Rate (GET /api/v2/mix/market/current-fund-rate)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetFundingRateRaw {
    pub symbol: String,
    #[serde(rename = "fundingRate")]
    pub funding_rate: String,
}

impl BitgetFundingRateRaw {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        FundingRate {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            rate: Decimal::from_str(&self.funding_rate).unwrap_or_default(),
            next_funding_time_ms: 0,
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Open Interest (GET /api/v2/mix/market/open-interest)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetOpenInterestRaw {
    pub symbol: String,
    #[serde(rename = "openInterest")]
    pub open_interest: String,
    pub ts: String,
}

impl BitgetOpenInterestRaw {
    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = bitget_symbol_to_unified(&self.symbol);
        let oi = Decimal::from_str(&self.open_interest).unwrap_or_default();
        OpenInterest {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            open_interest: oi,
            open_interest_value: Decimal::ZERO,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Klines (GET /api/v2/mix/market/candles)
// ---------------------------------------------------------------------------

/// Parse a single Bitget futures kline row into a Candle.
///
/// Row format: [ts, open, high, low, close, baseVol, quoteVol, usdtVol]
pub fn parse_kline_row(row: &[String], symbol: Symbol) -> Option<Candle> {
    if row.len() < 6 {
        return None;
    }
    let open_time_ms = row[0].parse::<u64>().ok()?;
    let open = Decimal::from_str(&row[1]).ok()?;
    let high = Decimal::from_str(&row[2]).ok()?;
    let low = Decimal::from_str(&row[3]).ok()?;
    let close = Decimal::from_str(&row[4]).ok()?;
    let volume = Decimal::from_str(&row[5]).ok()?;

    Some(Candle {
        exchange: ExchangeId::BitgetFutures,
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
// WebSocket types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixWsTradeRaw {
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    pub side: String,
    pub price: String,
    pub size: String,
    pub ts: String,
}

impl BitgetMixWsTradeRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let side = match self.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Trade {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            side,
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BitgetMixWsOrderBook {
    pub asks: Vec<[String; 2]>,
    pub bids: Vec<[String; 2]>,
    pub ts: String,
    #[serde(default)]
    pub seq: Option<String>,
}

impl BitgetMixWsOrderBook {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        let seq = self.seq.as_deref().and_then(|s| s.parse::<u64>().ok());
        OrderBook {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            bids: parse_levels(&self.bids),
            asks: parse_levels(&self.asks),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
            sequence: seq,
        }
    }
}

/// Parse a WS candle array: [ts, open, high, low, close, baseVol, quoteVol, usdtVol]
pub fn parse_ws_kline(row: &[String], symbol: Symbol) -> Option<Candle> {
    parse_kline_row(row, symbol)
}

// ---------------------------------------------------------------------------
// WS Ticker (for mark price streaming)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixWsTickerRaw {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
    #[serde(rename = "indexPrice")]
    pub index_price: String,
    pub ts: String,
}

impl BitgetMixWsTickerRaw {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = bitget_symbol_to_unified(&self.inst_id);
        MarkPrice {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: self.ts.parse::<u64>().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// WS Liquidation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BitgetMixWsLiquidationRaw {
    #[serde(rename = "instId")]
    pub inst_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    #[serde(rename = "updatedTime")]
    pub updated_time: String,
}

impl BitgetMixWsLiquidationRaw {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = bitget_symbol_to_unified(&self.inst_id);
        let side = match self.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            _ => Side::Sell,
        };
        Liquidation {
            exchange: ExchangeId::BitgetFutures,
            symbol,
            side,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from_str(&self.size).unwrap_or_default(),
            timestamp_ms: self.updated_time.parse::<u64>().unwrap_or(0),
        }
    }
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
    fn test_contracts_to_exchange_info() {
        let contracts = vec![
            BitgetMixContractRaw {
                symbol: "BTCUSDT".to_string(),
                base_coin: "BTC".to_string(),
                quote_coin: "USDT".to_string(),
                price_precision: "2".to_string(),
                quantity_precision: "6".to_string(),
                status: "online".to_string(),
                min_trade_usdt: "5".to_string(),
            },
            BitgetMixContractRaw {
                symbol: "ETHUSDT".to_string(),
                base_coin: "ETH".to_string(),
                quote_coin: "USDT".to_string(),
                price_precision: "2".to_string(),
                quantity_precision: "4".to_string(),
                status: "halt".to_string(),
                min_trade_usdt: "1".to_string(),
            },
        ];

        let info = contracts_to_exchange_info(contracts);
        assert_eq!(info.exchange, ExchangeId::BitgetFutures);
        assert_eq!(info.symbols.len(), 2);

        let btc = &info.symbols[0];
        assert_eq!(btc.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(btc.raw_symbol, "BTCUSDT");
        assert_eq!(btc.status, SymbolStatus::Trading);
        assert_eq!(btc.base_precision, 6);
        assert_eq!(btc.quote_precision, 2);
        assert_eq!(btc.min_notional, Some(dec!(5)));

        let eth = &info.symbols[1];
        assert_eq!(eth.status, SymbolStatus::Halted);
        assert_eq!(eth.min_notional, Some(dec!(1)));
    }

    #[test]
    fn test_orderbook_conversion() {
        let raw: BitgetMixOrderBookData = serde_json::from_str(
            r#"{
                "asks": [["50001.00", "2.0"]],
                "bids": [["50000.00", "1.0"], ["49999.00", "0.5"]],
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::BitgetFutures);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(1.0));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(2.0));
        assert!(ob.sequence.is_none());
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_conversion() {
        let raw: BitgetMixTradeRaw = serde_json::from_str(
            r#"{
                "tradeId": "abc123",
                "symbol": "ETHUSDT",
                "price": "2000.50",
                "size": "0.5",
                "side": "buy",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::BitgetFutures);
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.50));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.trade_id, Some("abc123".to_string()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_trade_sell_side() {
        let raw = BitgetMixTradeRaw {
            trade_id: "def456".to_string(),
            symbol: "BTCUSDT".to_string(),
            price: "50000.00".to_string(),
            size: "0.01".to_string(),
            side: "sell".to_string(),
            ts: "1700000000001".to_string(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.exchange, ExchangeId::BitgetFutures);
    }

    #[test]
    fn test_ticker_conversion() {
        let raw: BitgetMixTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPr": "50000.00",
                "bidPr": "49999.00",
                "askPr": "50001.00",
                "baseVolume": "12345.678",
                "change24h": "0.025",
                "ts": "1700000000000",
                "fundingRate": "0.0001",
                "markPrice": "50000.50",
                "indexPrice": "49999.80",
                "openInterest": "1000.5"
            }"#,
        )
        .unwrap();

        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::BitgetFutures);
        assert_eq!(ticker.symbol.base, "BTC");
        assert_eq!(ticker.symbol.quote, "USDT");
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
        assert_eq!(ticker.volume_24h, dec!(12345.678));
        assert_eq!(ticker.price_change_pct_24h, Some(dec!(0.025)));
        assert_eq!(ticker.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ticker_into_mark_price() {
        let raw: BitgetMixTickerRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "lastPr": "50000.00",
                "bidPr": "49999.00",
                "askPr": "50001.00",
                "baseVolume": "12345.678",
                "change24h": "0.025",
                "ts": "1700000000000",
                "fundingRate": "0.0001",
                "markPrice": "50000.50",
                "indexPrice": "49999.80",
                "openInterest": "1000.5"
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::BitgetFutures);
        assert_eq!(mp.symbol.base, "BTC");
        assert_eq!(mp.symbol.quote, "USDT");
        assert_eq!(mp.mark_price, dec!(50000.50));
        assert_eq!(mp.index_price, dec!(49999.80));
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_funding_rate_conversion() {
        let raw: BitgetFundingRateRaw = serde_json::from_str(
            r#"{
                "symbol": "BTCUSDT",
                "fundingRate": "0.0001"
            }"#,
        )
        .unwrap();

        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::BitgetFutures);
        assert_eq!(fr.symbol.base, "BTC");
        assert_eq!(fr.symbol.quote, "USDT");
        assert_eq!(fr.rate, dec!(0.0001));
    }

    #[test]
    fn test_open_interest_conversion() {
        let raw: BitgetOpenInterestRaw = serde_json::from_str(
            r#"{
                "symbol": "ETHUSDT",
                "openInterest": "50000.25",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let oi = raw.into_open_interest();
        assert_eq!(oi.exchange, ExchangeId::BitgetFutures);
        assert_eq!(oi.symbol.base, "ETH");
        assert_eq!(oi.symbol.quote, "USDT");
        assert_eq!(oi.open_interest, dec!(50000.25));
        assert_eq!(oi.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_parse_kline_row() {
        let row = vec![
            "1700000000000".to_string(),
            "50000.00".to_string(),
            "50200.00".to_string(),
            "49900.00".to_string(),
            "50100.00".to_string(),
            "100.5".to_string(),
            "5025000.00".to_string(),
            "5025000.00".to_string(),
        ];

        let candle = parse_kline_row(&row, Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::BitgetFutures);
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
    fn test_ws_trade_conversion() {
        let raw: BitgetMixWsTradeRaw = serde_json::from_str(
            r#"{
                "tradeId": "trade-999",
                "price": "2000.00",
                "size": "0.5",
                "side": "sell",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let trade = raw.into_trade(Symbol::new("ETH", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::BitgetFutures);
        assert_eq!(trade.symbol.base, "ETH");
        assert_eq!(trade.symbol.quote, "USDT");
        assert_eq!(trade.price, dec!(2000.00));
        assert_eq!(trade.qty, dec!(0.5));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.trade_id, Some("trade-999".to_string()));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_orderbook_conversion() {
        let raw: BitgetMixWsOrderBook = serde_json::from_str(
            r#"{
                "asks": [["50001.00", "2.0"]],
                "bids": [["50000.00", "1.0"]],
                "ts": "1700000000000",
                "seq": "100"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::BitgetFutures);
        assert_eq!(ob.symbol.base, "BTC");
        assert_eq!(ob.symbol.quote, "USDT");
        assert_eq!(ob.bids.len(), 1);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(100));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_orderbook_no_seq() {
        let raw: BitgetMixWsOrderBook = serde_json::from_str(
            r#"{
                "asks": [],
                "bids": [],
                "ts": "0"
            }"#,
        )
        .unwrap();

        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert!(ob.sequence.is_none());
    }

    #[test]
    fn test_ws_ticker_into_mark_price() {
        let raw: BitgetMixWsTickerRaw = serde_json::from_str(
            r#"{
                "instId": "BTCUSDT",
                "markPrice": "50000.50",
                "indexPrice": "49999.80",
                "ts": "1700000000000"
            }"#,
        )
        .unwrap();

        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::BitgetFutures);
        assert_eq!(mp.symbol.base, "BTC");
        assert_eq!(mp.mark_price, dec!(50000.50));
        assert_eq!(mp.index_price, dec!(49999.80));
        assert_eq!(mp.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_liquidation_conversion() {
        let raw: BitgetMixWsLiquidationRaw = serde_json::from_str(
            r#"{
                "instId": "BTCUSDT",
                "price": "50000",
                "size": "0.5",
                "side": "buy",
                "updatedTime": "1700000000000"
            }"#,
        )
        .unwrap();

        let liq = raw.into_liquidation();
        assert_eq!(liq.exchange, ExchangeId::BitgetFutures);
        assert_eq!(liq.symbol.base, "BTC");
        assert_eq!(liq.symbol.quote, "USDT");
        assert_eq!(liq.side, Side::Buy);
        assert_eq!(liq.price, dec!(50000));
        assert_eq!(liq.qty, dec!(0.5));
        assert_eq!(liq.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_ws_liquidation_sell_side() {
        let raw = BitgetMixWsLiquidationRaw {
            inst_id: "ETHUSDT".to_string(),
            price: "2000.00".to_string(),
            size: "1.0".to_string(),
            side: "sell".to_string(),
            updated_time: "1700000000001".to_string(),
        };
        let liq = raw.into_liquidation();
        assert_eq!(liq.side, Side::Sell);
        assert_eq!(liq.symbol.base, "ETH");
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
}
