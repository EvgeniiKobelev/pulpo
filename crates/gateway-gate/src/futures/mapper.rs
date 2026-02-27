use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// Re-use spot helpers for symbol/interval conversion.
pub use crate::spot::mapper::{gate_pair_to_unified, unified_to_gate};

const EXCHANGE: ExchangeId = ExchangeId::GateFutures;

// ---------------------------------------------------------------------------
// Contracts (GET /futures/usdt/contracts)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesContractRaw {
    pub name: String,
    #[serde(default)]
    pub mark_price: String,
    #[serde(default)]
    pub index_price: String,
    #[serde(default)]
    pub funding_rate: String,
    #[serde(default)]
    pub funding_next_apply: Option<f64>,
    #[serde(default)]
    pub funding_interval: Option<u64>,
    #[serde(default)]
    pub order_price_round: String,
    #[serde(default)]
    pub order_size_min: Option<i64>,
    #[serde(default)]
    pub order_size_max: Option<i64>,
    #[serde(default)]
    pub quanto_multiplier: String,
    #[serde(default)]
    pub last_price: String,
    #[serde(default)]
    pub position_size: Option<i64>,
    #[serde(default)]
    pub status: String,
}

pub fn contracts_to_exchange_info(contracts: Vec<GateFuturesContractRaw>) -> ExchangeInfo {
    let symbols = contracts
        .into_iter()
        .map(|c| {
            let symbol = gate_pair_to_unified(&c.name);
            let status = futures_status_to_unified(&c.status);
            let tick_size = Decimal::from_str(&c.order_price_round).ok();
            let min_qty = c.order_size_min.map(Decimal::from);

            SymbolInfo {
                symbol,
                raw_symbol: c.name,
                status,
                base_precision: tick_size
                    .map(|t| t.scale() as u8)
                    .unwrap_or(0),
                quote_precision: 0,
                min_qty,
                min_notional: None,
                tick_size,
            }
        })
        .collect();

    ExchangeInfo {
        exchange: EXCHANGE,
        symbols,
    }
}

// ---------------------------------------------------------------------------
// OrderBook (GET /futures/usdt/order_book)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesOrderBookItem {
    pub p: String,
    pub s: i64,
}

#[derive(Debug, Deserialize)]
pub struct GateFuturesOrderBookRaw {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub current: Option<f64>,
    #[serde(default)]
    pub update: Option<f64>,
    pub asks: Vec<GateFuturesOrderBookItem>,
    pub bids: Vec<GateFuturesOrderBookItem>,
}

impl GateFuturesOrderBookRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        let ts = self
            .current
            .or(self.update)
            .map(|t| (t * 1000.0) as u64)
            .unwrap_or(0);
        OrderBook {
            exchange: EXCHANGE,
            symbol,
            bids: parse_futures_levels(&self.bids),
            asks: parse_futures_levels(&self.asks),
            timestamp_ms: ts,
            sequence: self.id,
        }
    }
}

// ---------------------------------------------------------------------------
// Trades (GET /futures/usdt/trades)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesTradeRaw {
    pub id: u64,
    #[serde(default)]
    pub create_time_ms: Option<f64>,
    #[serde(default)]
    pub create_time: Option<f64>,
    pub contract: String,
    pub size: i64,
    pub price: String,
}

impl GateFuturesTradeRaw {
    pub fn into_trade(self) -> Trade {
        let symbol = gate_pair_to_unified(&self.contract);
        let ts = self
            .create_time_ms
            .map(|t| t as u64)
            .or_else(|| self.create_time.map(|t| (t * 1000.0) as u64))
            .unwrap_or(0);
        let side = if self.size >= 0 {
            Side::Buy
        } else {
            Side::Sell
        };
        Trade {
            exchange: EXCHANGE,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from(self.size.unsigned_abs()),
            side,
            timestamp_ms: ts,
            trade_id: Some(self.id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tickers (GET /futures/usdt/tickers)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesTickerRaw {
    pub contract: String,
    pub last: String,
    #[serde(default)]
    pub change_percentage: String,
    #[serde(default)]
    pub total_size: String,
    #[serde(default)]
    pub volume_24h: String,
    #[serde(default)]
    pub volume_24h_base: String,
    #[serde(default)]
    pub mark_price: String,
    #[serde(default)]
    pub index_price: String,
    #[serde(default)]
    pub funding_rate: String,
    #[serde(default)]
    pub funding_rate_indicative: String,
    #[serde(default)]
    pub lowest_ask: String,
    #[serde(default)]
    pub highest_bid: String,
    #[serde(default)]
    pub high_24h: String,
    #[serde(default)]
    pub low_24h: String,
}

impl GateFuturesTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = gate_pair_to_unified(&self.contract);
        Ticker {
            exchange: EXCHANGE,
            symbol,
            last_price: Decimal::from_str(&self.last).unwrap_or_default(),
            bid: Decimal::from_str(&self.highest_bid).ok(),
            ask: Decimal::from_str(&self.lowest_ask).ok(),
            volume_24h: Decimal::from_str(&self.volume_24h).unwrap_or_default(),
            price_change_pct_24h: Decimal::from_str(&self.change_percentage).ok(),
            timestamp_ms: 0,
        }
    }

    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = gate_pair_to_unified(&self.contract);
        FundingRate {
            exchange: EXCHANGE,
            symbol,
            rate: Decimal::from_str(&self.funding_rate).unwrap_or_default(),
            next_funding_time_ms: 0,
            timestamp_ms: 0,
        }
    }

    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = gate_pair_to_unified(&self.contract);
        MarkPrice {
            exchange: EXCHANGE,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).unwrap_or_default(),
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: 0,
        }
    }

    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = gate_pair_to_unified(&self.contract);
        OpenInterest {
            exchange: EXCHANGE,
            symbol,
            open_interest: Decimal::from_str(&self.total_size).unwrap_or_default(),
            open_interest_value: Decimal::ZERO,
            timestamp_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Candlesticks (GET /futures/usdt/candlesticks)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesCandleRaw {
    pub t: f64,
    #[serde(default)]
    pub v: Option<i64>,
    pub c: String,
    pub h: String,
    pub l: String,
    pub o: String,
    #[serde(default)]
    pub sum: String,
}

impl GateFuturesCandleRaw {
    pub fn into_candle(self, symbol: Symbol) -> Option<Candle> {
        Some(Candle {
            exchange: EXCHANGE,
            symbol,
            open: Decimal::from_str(&self.o).ok()?,
            high: Decimal::from_str(&self.h).ok()?,
            low: Decimal::from_str(&self.l).ok()?,
            close: Decimal::from_str(&self.c).ok()?,
            volume: self.v.map(|v| Decimal::from(v)).unwrap_or_default(),
            open_time_ms: (self.t as u64) * 1000,
            close_time_ms: 0,
            is_closed: true,
        })
    }
}

// ---------------------------------------------------------------------------
// Liquidation Orders (GET /futures/usdt/liq_orders)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesLiqOrderRaw {
    pub time: u64,
    pub contract: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub order_size: i64,
    pub order_price: String,
    #[serde(default)]
    pub fill_price: String,
    #[serde(default)]
    pub left: i64,
}

impl GateFuturesLiqOrderRaw {
    pub fn into_liquidation(self) -> Liquidation {
        let symbol = gate_pair_to_unified(&self.contract);
        let side = if self.size >= 0 {
            Side::Sell // long position liquidated → sell
        } else {
            Side::Buy // short position liquidated → buy
        };
        let price = Decimal::from_str(&self.fill_price)
            .or_else(|_| Decimal::from_str(&self.order_price))
            .unwrap_or_default();
        Liquidation {
            exchange: EXCHANGE,
            symbol,
            side,
            price,
            qty: Decimal::from(self.order_size.unsigned_abs()),
            timestamp_ms: self.time * 1000,
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GateFuturesWsTradeResult {
    pub id: Option<u64>,
    #[serde(default)]
    pub create_time_ms: Option<f64>,
    pub contract: String,
    pub size: i64,
    pub price: String,
}

impl GateFuturesWsTradeResult {
    pub fn into_trade(self) -> Trade {
        let symbol = gate_pair_to_unified(&self.contract);
        let ts = self.create_time_ms.map(|t| t as u64).unwrap_or(0);
        let side = if self.size >= 0 {
            Side::Buy
        } else {
            Side::Sell
        };
        Trade {
            exchange: EXCHANGE,
            symbol,
            price: Decimal::from_str(&self.price).unwrap_or_default(),
            qty: Decimal::from(self.size.unsigned_abs()),
            side,
            timestamp_ms: ts,
            trade_id: self.id.map(|i| i.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GateFuturesWsOrderBookResult {
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub contract: Option<String>,
    pub asks: Vec<GateFuturesOrderBookItem>,
    pub bids: Vec<GateFuturesOrderBookItem>,
}

impl GateFuturesWsOrderBookResult {
    pub fn into_orderbook(self, fallback_symbol: Symbol) -> OrderBook {
        let symbol = self
            .contract
            .as_deref()
            .map(gate_pair_to_unified)
            .unwrap_or(fallback_symbol);
        OrderBook {
            exchange: EXCHANGE,
            symbol,
            bids: parse_futures_levels(&self.bids),
            asks: parse_futures_levels(&self.asks),
            timestamp_ms: self.t.unwrap_or(0),
            sequence: self.id,
        }
    }
}

/// Gate.io futures WS candle result.
///
/// Fields: t=timestamp(s), v=quote_vol, c=close, h=high, l=low, o=open, n=name, a=base_vol
#[derive(Debug, Deserialize)]
pub struct GateFuturesWsCandleResult {
    pub t: String,
    #[serde(default)]
    pub v: String,
    pub c: String,
    pub h: String,
    pub l: String,
    pub o: String,
    pub n: String,
    #[serde(default)]
    pub a: String,
}

impl GateFuturesWsCandleResult {
    pub fn into_candle(self) -> Option<Candle> {
        // n format: "1m_BTC_USDT"
        let pair = self.n.splitn(2, '_').nth(1)?;
        let symbol = gate_pair_to_unified(pair);
        let open_time_secs: u64 = self.t.parse().ok()?;
        Some(Candle {
            exchange: EXCHANGE,
            symbol,
            open: Decimal::from_str(&self.o).ok()?,
            high: Decimal::from_str(&self.h).ok()?,
            low: Decimal::from_str(&self.l).ok()?,
            close: Decimal::from_str(&self.c).ok()?,
            volume: Decimal::from_str(&self.a).unwrap_or_default(),
            open_time_ms: open_time_secs * 1000,
            close_time_ms: 0,
            is_closed: false,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GateFuturesWsTickerResult {
    pub contract: String,
    pub last: String,
    #[serde(default)]
    pub mark_price: String,
    #[serde(default)]
    pub index_price: String,
    #[serde(default)]
    pub funding_rate: String,
    #[serde(default)]
    pub total_size: String,
    #[serde(default)]
    pub volume_24h: String,
    #[serde(default)]
    pub change_percentage: String,
    #[serde(default)]
    pub lowest_ask: String,
    #[serde(default)]
    pub highest_bid: String,
}

impl GateFuturesWsTickerResult {
    pub fn into_mark_price(self) -> Option<MarkPrice> {
        let symbol = gate_pair_to_unified(&self.contract);
        Some(MarkPrice {
            exchange: EXCHANGE,
            symbol,
            mark_price: Decimal::from_str(&self.mark_price).ok()?,
            index_price: Decimal::from_str(&self.index_price).unwrap_or_default(),
            timestamp_ms: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Parse futures order-book levels from `{p, s}` objects.
pub fn parse_futures_levels(items: &[GateFuturesOrderBookItem]) -> Vec<Level> {
    items
        .iter()
        .filter_map(|item| {
            let price = Decimal::from_str(&item.p).ok()?;
            let qty = Decimal::from(item.s.unsigned_abs());
            Some(Level::new(price, qty))
        })
        .collect()
}

/// Map a unified Interval to the Gate.io futures interval string.
pub fn interval_to_gate_futures(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "10s",
        Interval::M1 => "1m",
        Interval::M3 => "5m",
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "1h",
        Interval::H4 => "4h",
        Interval::D1 => "1d",
        Interval::W1 => "7d",
    }
}

/// Map a Gate.io futures contract status to unified SymbolStatus.
fn futures_status_to_unified(status: &str) -> SymbolStatus {
    match status {
        "trading" => SymbolStatus::Trading,
        "delisting" | "delisted" => SymbolStatus::Halted,
        "prelaunch" => SymbolStatus::PreTrading,
        _ => SymbolStatus::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_futures_trade_buy() {
        let raw = GateFuturesTradeRaw {
            id: 12345,
            create_time_ms: Some(1700000000123.0),
            create_time: Some(1700000000.0),
            contract: "BTC_USDT".into(),
            size: 100,
            price: "50000.50".into(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::GateFutures);
        assert_eq!(trade.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(trade.price, dec!(50000.50));
        assert_eq!(trade.qty, dec!(100));
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.timestamp_ms, 1700000000123);
    }

    #[test]
    fn test_futures_trade_sell() {
        let raw = GateFuturesTradeRaw {
            id: 12346,
            create_time_ms: None,
            create_time: Some(1700000000.0),
            contract: "ETH_USDT".into(),
            size: -50,
            price: "2000.00".into(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.qty, dec!(50));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_futures_orderbook_conversion() {
        let raw = GateFuturesOrderBookRaw {
            id: Some(42),
            current: Some(1700000000.123),
            update: None,
            asks: vec![
                GateFuturesOrderBookItem { p: "50001.00".into(), s: 200 },
            ],
            bids: vec![
                GateFuturesOrderBookItem { p: "50000.00".into(), s: 100 },
                GateFuturesOrderBookItem { p: "49999.00".into(), s: 50 },
            ],
        };
        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::GateFutures);
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.bids[0].qty, dec!(100));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.asks[0].qty, dec!(200));
        assert_eq!(ob.sequence, Some(42));
        assert_eq!(ob.timestamp_ms, 1700000000123);
    }

    #[test]
    fn test_futures_ticker_conversion() {
        let raw = GateFuturesTickerRaw {
            contract: "BTC_USDT".into(),
            last: "50000.00".into(),
            change_percentage: "2.5".into(),
            total_size: "1000000".into(),
            volume_24h: "50000".into(),
            volume_24h_base: "".into(),
            mark_price: "50010.00".into(),
            index_price: "50005.00".into(),
            funding_rate: "0.0001".into(),
            funding_rate_indicative: "0.00012".into(),
            lowest_ask: "50001.00".into(),
            highest_bid: "49999.00".into(),
            high_24h: "52000.00".into(),
            low_24h: "48000.00".into(),
        };
        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::GateFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000.00));
        assert_eq!(ticker.bid, Some(dec!(49999.00)));
        assert_eq!(ticker.ask, Some(dec!(50001.00)));
    }

    #[test]
    fn test_futures_ticker_to_funding_rate() {
        let raw = GateFuturesTickerRaw {
            contract: "BTC_USDT".into(),
            last: "50000".into(),
            change_percentage: "".into(),
            total_size: "".into(),
            volume_24h: "".into(),
            volume_24h_base: "".into(),
            mark_price: "50010".into(),
            index_price: "50005".into(),
            funding_rate: "0.0001".into(),
            funding_rate_indicative: "".into(),
            lowest_ask: "".into(),
            highest_bid: "".into(),
            high_24h: "".into(),
            low_24h: "".into(),
        };
        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::GateFutures);
        assert_eq!(fr.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(fr.rate, dec!(0.0001));
    }

    #[test]
    fn test_futures_ticker_to_mark_price() {
        let raw = GateFuturesTickerRaw {
            contract: "ETH_USDT".into(),
            last: "2000".into(),
            change_percentage: "".into(),
            total_size: "".into(),
            volume_24h: "".into(),
            volume_24h_base: "".into(),
            mark_price: "2001.50".into(),
            index_price: "2000.75".into(),
            funding_rate: "".into(),
            funding_rate_indicative: "".into(),
            lowest_ask: "".into(),
            highest_bid: "".into(),
            high_24h: "".into(),
            low_24h: "".into(),
        };
        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::GateFutures);
        assert_eq!(mp.symbol, Symbol::new("ETH", "USDT"));
        assert_eq!(mp.mark_price, dec!(2001.50));
        assert_eq!(mp.index_price, dec!(2000.75));
    }

    #[test]
    fn test_futures_candle_rest_conversion() {
        let raw = GateFuturesCandleRaw {
            t: 1700000000.0,
            v: Some(500),
            c: "50100.00".into(),
            h: "50200.00".into(),
            l: "49900.00".into(),
            o: "50000.00".into(),
            sum: "25000000".into(),
        };
        let candle = raw.into_candle(Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::GateFutures);
        assert_eq!(candle.open, dec!(50000.00));
        assert_eq!(candle.high, dec!(50200.00));
        assert_eq!(candle.low, dec!(49900.00));
        assert_eq!(candle.close, dec!(50100.00));
        assert_eq!(candle.volume, dec!(500));
        assert_eq!(candle.open_time_ms, 1700000000000);
    }

    #[test]
    fn test_futures_liq_order_conversion() {
        let raw = GateFuturesLiqOrderRaw {
            time: 1700000000,
            contract: "BTC_USDT".into(),
            size: 100,
            order_size: 100,
            order_price: "50000.00".into(),
            fill_price: "49990.00".into(),
            left: 0,
        };
        let liq = raw.into_liquidation();
        assert_eq!(liq.exchange, ExchangeId::GateFutures);
        assert_eq!(liq.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(liq.side, Side::Sell); // long liquidated
        assert_eq!(liq.price, dec!(49990.00));
        assert_eq!(liq.qty, dec!(100));
        assert_eq!(liq.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_futures_liq_order_short() {
        let raw = GateFuturesLiqOrderRaw {
            time: 1700000000,
            contract: "ETH_USDT".into(),
            size: -50,
            order_size: 50,
            order_price: "2000.00".into(),
            fill_price: "2010.00".into(),
            left: 0,
        };
        let liq = raw.into_liquidation();
        assert_eq!(liq.side, Side::Buy); // short liquidated
    }

    #[test]
    fn test_parse_futures_levels() {
        let items = vec![
            GateFuturesOrderBookItem { p: "100.50".into(), s: 10 },
            GateFuturesOrderBookItem { p: "99.00".into(), s: 20 },
        ];
        let levels = parse_futures_levels(&items);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].price, dec!(100.50));
        assert_eq!(levels[0].qty, dec!(10));
        assert_eq!(levels[1].price, dec!(99.00));
        assert_eq!(levels[1].qty, dec!(20));
    }

    #[test]
    fn test_parse_futures_levels_skips_invalid() {
        let items = vec![
            GateFuturesOrderBookItem { p: "bad".into(), s: 10 },
            GateFuturesOrderBookItem { p: "50.00".into(), s: 30 },
        ];
        let levels = parse_futures_levels(&items);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].price, dec!(50.00));
    }

    #[test]
    fn test_futures_status() {
        assert_eq!(futures_status_to_unified("trading"), SymbolStatus::Trading);
        assert_eq!(futures_status_to_unified("delisting"), SymbolStatus::Halted);
        assert_eq!(futures_status_to_unified("delisted"), SymbolStatus::Halted);
        assert_eq!(futures_status_to_unified("prelaunch"), SymbolStatus::PreTrading);
        assert_eq!(futures_status_to_unified("unknown"), SymbolStatus::Unknown);
    }

    #[test]
    fn test_interval_to_gate_futures() {
        assert_eq!(interval_to_gate_futures(Interval::S1), "10s");
        assert_eq!(interval_to_gate_futures(Interval::M1), "1m");
        assert_eq!(interval_to_gate_futures(Interval::M5), "5m");
        assert_eq!(interval_to_gate_futures(Interval::M15), "15m");
        assert_eq!(interval_to_gate_futures(Interval::M30), "30m");
        assert_eq!(interval_to_gate_futures(Interval::H1), "1h");
        assert_eq!(interval_to_gate_futures(Interval::H4), "4h");
        assert_eq!(interval_to_gate_futures(Interval::D1), "1d");
        assert_eq!(interval_to_gate_futures(Interval::W1), "7d");
    }

    #[test]
    fn test_contracts_to_exchange_info() {
        let contracts = vec![
            GateFuturesContractRaw {
                name: "BTC_USDT".into(),
                mark_price: "50000".into(),
                index_price: "49990".into(),
                funding_rate: "0.0001".into(),
                funding_next_apply: Some(1700000000.0),
                funding_interval: Some(28800),
                order_price_round: "0.1".into(),
                order_size_min: Some(1),
                order_size_max: Some(1000000),
                quanto_multiplier: "0.0001".into(),
                last_price: "50000".into(),
                position_size: Some(100000),
                status: "trading".into(),
            },
        ];
        let info = contracts_to_exchange_info(contracts);
        assert_eq!(info.exchange, ExchangeId::GateFutures);
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.symbols[0].raw_symbol, "BTC_USDT");
        assert_eq!(info.symbols[0].status, SymbolStatus::Trading);
        assert_eq!(info.symbols[0].min_qty, Some(dec!(1)));
        assert_eq!(info.symbols[0].tick_size, Some(dec!(0.1)));
    }

    #[test]
    fn test_ws_trade_conversion() {
        let raw = GateFuturesWsTradeResult {
            id: Some(309143071),
            create_time_ms: Some(1700000000123.0),
            contract: "BTC_USDT".into(),
            size: -50,
            price: "50000.00".into(),
        };
        let trade = raw.into_trade();
        assert_eq!(trade.exchange, ExchangeId::GateFutures);
        assert_eq!(trade.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(trade.side, Side::Sell);
        assert_eq!(trade.qty, dec!(50));
        assert_eq!(trade.timestamp_ms, 1700000000123);
    }

    #[test]
    fn test_ws_orderbook_conversion() {
        let raw = GateFuturesWsOrderBookResult {
            t: Some(1700000000000),
            id: Some(48791820),
            contract: Some("BTC_USDT".into()),
            asks: vec![GateFuturesOrderBookItem { p: "50001.00".into(), s: 200 }],
            bids: vec![GateFuturesOrderBookItem { p: "50000.00".into(), s: 100 }],
        };
        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::GateFutures);
        assert_eq!(ob.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ob.bids[0].price, dec!(50000.00));
        assert_eq!(ob.asks[0].price, dec!(50001.00));
        assert_eq!(ob.sequence, Some(48791820));
    }

    #[test]
    fn test_ws_candle_conversion() {
        let raw = GateFuturesWsCandleResult {
            t: "1606292580".into(),
            v: "2362.32035".into(),
            c: "19128.1".into(),
            h: "19130.0".into(),
            l: "19125.0".into(),
            o: "19126.5".into(),
            n: "1m_BTC_USDT".into(),
            a: "3.8283".into(),
        };
        let candle = raw.into_candle().unwrap();
        assert_eq!(candle.exchange, ExchangeId::GateFutures);
        assert_eq!(candle.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(candle.open, dec!(19126.5));
        assert_eq!(candle.close, dec!(19128.1));
        assert_eq!(candle.open_time_ms, 1606292580000);
    }

    #[test]
    fn test_ws_ticker_to_mark_price() {
        let raw = GateFuturesWsTickerResult {
            contract: "BTC_USDT".into(),
            last: "50000".into(),
            mark_price: "50010.50".into(),
            index_price: "50005.25".into(),
            funding_rate: "0.0001".into(),
            total_size: "".into(),
            volume_24h: "".into(),
            change_percentage: "".into(),
            lowest_ask: "".into(),
            highest_bid: "".into(),
        };
        let mp = raw.into_mark_price().unwrap();
        assert_eq!(mp.exchange, ExchangeId::GateFutures);
        assert_eq!(mp.mark_price, dec!(50010.50));
        assert_eq!(mp.index_price, dec!(50005.25));
    }
}
