use gateway_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;

const EXCHANGE: ExchangeId = ExchangeId::MexcFutures;

// ---------------------------------------------------------------------------
// Symbol helpers
// ---------------------------------------------------------------------------

/// Convert a unified Symbol to MEXC futures format: `"BTC_USDT"`.
pub fn unified_to_mexc_futures(symbol: &Symbol) -> String {
    format!("{}_{}", symbol.base, symbol.quote)
}

/// Convert a MEXC futures symbol `"BTC_USDT"` to a unified Symbol.
pub fn mexc_futures_to_unified(raw: &str) -> Symbol {
    if let Some((base, quote)) = raw.split_once('_') {
        Symbol::new(base, quote)
    } else {
        Symbol::new(raw, "")
    }
}

/// Map a unified Interval to the MEXC futures kline interval string.
pub fn interval_to_mexc_futures(interval: Interval) -> &'static str {
    match interval {
        Interval::S1 => "Min1",
        Interval::M1 => "Min1",
        Interval::M3 => "Min5",
        Interval::M5 => "Min5",
        Interval::M15 => "Min15",
        Interval::M30 => "Min30",
        Interval::H1 => "Min60",
        Interval::H4 => "Hour4",
        Interval::D1 => "Day1",
        Interval::W1 => "Week1",
    }
}

// ---------------------------------------------------------------------------
// Contract response wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MexcContractResponse<T> {
    pub success: bool,
    pub code: i32,
    pub data: T,
}

// ---------------------------------------------------------------------------
// Contract detail (GET /api/v1/contract/detail)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcContractDetailRaw {
    pub symbol: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub base_coin: Option<String>,
    #[serde(default)]
    pub quote_coin: Option<String>,
    #[serde(default)]
    pub settle_coin: Option<String>,
    #[serde(default)]
    pub contract_size: Option<f64>,
    #[serde(default)]
    pub min_leverage: Option<f64>,
    #[serde(default)]
    pub max_leverage: Option<f64>,
    #[serde(default)]
    pub price_scale: Option<u8>,
    #[serde(default)]
    pub vol_scale: Option<u8>,
    #[serde(default)]
    pub amount_scale: Option<u8>,
    #[serde(default)]
    pub price_unit: Option<f64>,
    #[serde(default)]
    pub vol_unit: Option<f64>,
    #[serde(default)]
    pub min_vol: Option<f64>,
    #[serde(default)]
    pub max_vol: Option<f64>,
    #[serde(default)]
    pub state: Option<i32>,
    #[serde(default)]
    pub is_new: Option<bool>,
    #[serde(default)]
    pub is_hot: Option<bool>,
    #[serde(default)]
    pub is_hidden: Option<bool>,
}

pub fn contracts_to_exchange_info(contracts: Vec<MexcContractDetailRaw>) -> ExchangeInfo {
    let symbols = contracts
        .into_iter()
        .filter(|c| c.state != Some(1)) // 0 = enabled, 1 = disabled
        .map(|c| {
            let symbol = mexc_futures_to_unified(&c.symbol);
            let status = match c.state {
                Some(0) => SymbolStatus::Trading,
                _ => SymbolStatus::Unknown,
            };
            let price_scale = c.price_scale.unwrap_or(0);
            let vol_scale = c.vol_scale.unwrap_or(0);
            let tick_size = c.price_unit.and_then(|u| Decimal::try_from(u).ok());
            let min_qty = c.min_vol.map(|v| Decimal::try_from(v).unwrap_or_default());

            SymbolInfo {
                symbol,
                raw_symbol: c.symbol,
                status,
                base_precision: price_scale,
                quote_precision: vol_scale,
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
// Depth (GET /api/v1/contract/depth/{symbol})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MexcFuturesDepthRaw {
    pub asks: Vec<MexcFuturesDepthLevel>,
    pub bids: Vec<MexcFuturesDepthLevel>,
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct MexcFuturesDepthLevel(pub f64, pub f64, pub f64);

impl MexcFuturesDepthRaw {
    pub fn into_orderbook(self, symbol: Symbol) -> OrderBook {
        OrderBook {
            exchange: EXCHANGE,
            symbol,
            bids: self
                .bids
                .iter()
                .filter_map(|l| {
                    let price = Decimal::try_from(l.0).ok()?;
                    let qty = Decimal::try_from(l.1).ok()?;
                    Some(Level::new(price, qty))
                })
                .collect(),
            asks: self
                .asks
                .iter()
                .filter_map(|l| {
                    let price = Decimal::try_from(l.0).ok()?;
                    let qty = Decimal::try_from(l.1).ok()?;
                    Some(Level::new(price, qty))
                })
                .collect(),
            timestamp_ms: self.timestamp.unwrap_or(0),
            sequence: self.version,
        }
    }
}

// ---------------------------------------------------------------------------
// Deals (GET /api/v1/contract/deals/{symbol})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MexcFuturesDealRaw {
    pub p: f64,
    pub v: f64,
    #[serde(alias = "T")]
    pub t_type: i32,
    pub t: u64,
    #[serde(alias = "O")]
    #[serde(default)]
    pub o: Option<i32>,
}

impl MexcFuturesDealRaw {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let side = if self.t_type == 1 {
            Side::Buy
        } else {
            Side::Sell
        };
        Trade {
            exchange: EXCHANGE,
            symbol,
            price: Decimal::try_from(self.p).unwrap_or_default(),
            qty: Decimal::try_from(self.v).unwrap_or_default(),
            side,
            timestamp_ms: self.t,
            trade_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Ticker (GET /api/v1/contract/ticker)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcFuturesTickerRaw {
    pub symbol: String,
    #[serde(default)]
    pub last_price: Option<f64>,
    #[serde(default)]
    pub bid1: Option<f64>,
    #[serde(default)]
    pub ask1: Option<f64>,
    #[serde(default)]
    pub volume24: Option<f64>,
    #[serde(default)]
    pub amount24: Option<f64>,
    #[serde(default)]
    pub hold_vol: Option<f64>,
    #[serde(default)]
    pub lower24_price: Option<f64>,
    #[serde(default)]
    pub upper24_price: Option<f64>,
    #[serde(default)]
    pub rise_fall_rate: Option<f64>,
    #[serde(default)]
    pub rise_fall_value: Option<f64>,
    #[serde(default)]
    pub index_price: Option<f64>,
    #[serde(default)]
    pub fair_price: Option<f64>,
    #[serde(default)]
    pub funding_rate: Option<f64>,
    #[serde(default)]
    pub max_bid_price: Option<f64>,
    #[serde(default)]
    pub min_ask_price: Option<f64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl MexcFuturesTickerRaw {
    pub fn into_ticker(self) -> Ticker {
        let symbol = mexc_futures_to_unified(&self.symbol);
        Ticker {
            exchange: EXCHANGE,
            symbol,
            last_price: self
                .last_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            bid: self.bid1.and_then(|v| Decimal::try_from(v).ok()),
            ask: self.ask1.and_then(|v| Decimal::try_from(v).ok()),
            volume_24h: self
                .volume24
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            price_change_pct_24h: self
                .rise_fall_rate
                .and_then(|v| Decimal::try_from(v).ok()),
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }

    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = mexc_futures_to_unified(&self.symbol);
        FundingRate {
            exchange: EXCHANGE,
            symbol,
            rate: self
                .funding_rate
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            next_funding_time_ms: 0,
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }

    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = mexc_futures_to_unified(&self.symbol);
        MarkPrice {
            exchange: EXCHANGE,
            symbol,
            mark_price: self
                .fair_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            index_price: self
                .index_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }

    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = mexc_futures_to_unified(&self.symbol);
        OpenInterest {
            exchange: EXCHANGE,
            symbol,
            open_interest: self
                .hold_vol
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            open_interest_value: Decimal::ZERO,
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Kline (GET /api/v1/contract/kline/{symbol})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MexcFuturesKlineRaw {
    pub time: Option<u64>,
    pub open: Option<f64>,
    pub close: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub vol: Option<f64>,
    pub amount: Option<f64>,
}

impl MexcFuturesKlineRaw {
    pub fn into_candle(self, symbol: Symbol) -> Option<Candle> {
        Some(Candle {
            exchange: EXCHANGE,
            symbol,
            open: Decimal::try_from(self.open?).ok()?,
            high: Decimal::try_from(self.high?).ok()?,
            low: Decimal::try_from(self.low?).ok()?,
            close: Decimal::try_from(self.close?).ok()?,
            volume: self
                .vol
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            open_time_ms: self.time.unwrap_or(0) * 1000,
            close_time_ms: 0,
            is_closed: true,
        })
    }
}

// ---------------------------------------------------------------------------
// Funding rate (GET /api/v1/contract/funding_rate/{symbol})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcFuturesFundingRateRaw {
    pub symbol: String,
    #[serde(default)]
    pub funding_rate: Option<f64>,
    #[serde(default)]
    pub max_funding_rate: Option<f64>,
    #[serde(default)]
    pub min_funding_rate: Option<f64>,
    #[serde(default)]
    pub collect_cycle: Option<u64>,
    #[serde(default)]
    pub next_settle_time: Option<u64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl MexcFuturesFundingRateRaw {
    pub fn into_funding_rate(self) -> FundingRate {
        let symbol = mexc_futures_to_unified(&self.symbol);
        FundingRate {
            exchange: EXCHANGE,
            symbol,
            rate: self
                .funding_rate
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            next_funding_time_ms: self.next_settle_time.unwrap_or(0),
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Fair price (GET /api/v1/contract/fair_price/{symbol})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcFuturesFairPriceRaw {
    pub symbol: String,
    #[serde(default)]
    pub fair_price: Option<f64>,
    #[serde(default)]
    pub index_price: Option<f64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl MexcFuturesFairPriceRaw {
    pub fn into_mark_price(self) -> MarkPrice {
        let symbol = mexc_futures_to_unified(&self.symbol);
        MarkPrice {
            exchange: EXCHANGE,
            symbol,
            mark_price: self
                .fair_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            index_price: self
                .index_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Open interest (GET /api/v1/contract/open_interest/{symbol})
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcFuturesOpenInterestRaw {
    pub symbol: String,
    #[serde(default)]
    pub hold_vol: Option<f64>,
    #[serde(default)]
    pub hold_coin: Option<f64>,
    #[serde(default)]
    pub hold_amount: Option<f64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl MexcFuturesOpenInterestRaw {
    pub fn into_open_interest(self) -> OpenInterest {
        let symbol = mexc_futures_to_unified(&self.symbol);
        OpenInterest {
            exchange: EXCHANGE,
            symbol,
            open_interest: self
                .hold_vol
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            open_interest_value: self
                .hold_amount
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            timestamp_ms: self.timestamp.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket types
// ---------------------------------------------------------------------------

/// Outer WS message envelope.
///
/// Format: `{"channel": "push.depth.full", "data": {...}, "symbol": "BTC_USDT", "ts": 123}`
#[derive(Debug, Deserialize)]
pub struct MexcFuturesWsMessage {
    pub channel: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub ts: Option<u64>,
}

/// WS depth full snapshot data.
#[derive(Debug, Deserialize)]
pub struct MexcFuturesWsDepthFull {
    pub asks: Vec<MexcFuturesDepthLevel>,
    pub bids: Vec<MexcFuturesDepthLevel>,
    #[serde(default)]
    pub version: Option<u64>,
}

impl MexcFuturesWsDepthFull {
    pub fn into_orderbook(self, symbol: Symbol, timestamp_ms: u64) -> OrderBook {
        OrderBook {
            exchange: EXCHANGE,
            symbol,
            bids: self
                .bids
                .iter()
                .filter_map(|l| {
                    let price = Decimal::try_from(l.0).ok()?;
                    let qty = Decimal::try_from(l.1).ok()?;
                    Some(Level::new(price, qty))
                })
                .collect(),
            asks: self
                .asks
                .iter()
                .filter_map(|l| {
                    let price = Decimal::try_from(l.0).ok()?;
                    let qty = Decimal::try_from(l.1).ok()?;
                    Some(Level::new(price, qty))
                })
                .collect(),
            timestamp_ms,
            sequence: self.version,
        }
    }
}

/// WS deal (trade) data — single item.
#[derive(Debug, Deserialize)]
pub struct MexcFuturesWsDeal {
    pub p: f64,
    pub v: f64,
    #[serde(alias = "T")]
    pub t_type: i32,
    pub t: u64,
    #[serde(alias = "O")]
    #[serde(default)]
    pub o: Option<i32>,
    #[serde(default, alias = "M")]
    pub m: Option<i32>,
}

impl MexcFuturesWsDeal {
    pub fn into_trade(self, symbol: Symbol) -> Trade {
        let side = if self.t_type == 1 {
            Side::Buy
        } else {
            Side::Sell
        };
        Trade {
            exchange: EXCHANGE,
            symbol,
            price: Decimal::try_from(self.p).unwrap_or_default(),
            qty: Decimal::try_from(self.v).unwrap_or_default(),
            side,
            timestamp_ms: self.t,
            trade_id: None,
        }
    }
}

/// WS kline data.
#[derive(Debug, Deserialize)]
pub struct MexcFuturesWsKline {
    #[serde(default)]
    pub t: Option<u64>,
    #[serde(default)]
    pub o: Option<f64>,
    #[serde(default)]
    pub c: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
    #[serde(default)]
    pub l: Option<f64>,
    #[serde(default)]
    pub q: Option<f64>,
}

impl MexcFuturesWsKline {
    pub fn into_candle(self, symbol: Symbol) -> Option<Candle> {
        Some(Candle {
            exchange: EXCHANGE,
            symbol,
            open: Decimal::try_from(self.o?).ok()?,
            high: Decimal::try_from(self.h?).ok()?,
            low: Decimal::try_from(self.l?).ok()?,
            close: Decimal::try_from(self.c?).ok()?,
            volume: self
                .q
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            open_time_ms: self.t.unwrap_or(0) * 1000,
            close_time_ms: 0,
            is_closed: false,
        })
    }
}

/// WS ticker data (contains mark price, index price, funding rate).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MexcFuturesWsTicker {
    pub symbol: String,
    #[serde(default)]
    pub last_price: Option<f64>,
    #[serde(default)]
    pub bid1: Option<f64>,
    #[serde(default)]
    pub ask1: Option<f64>,
    #[serde(default)]
    pub volume24: Option<f64>,
    #[serde(default)]
    pub hold_vol: Option<f64>,
    #[serde(default)]
    pub lower24_price: Option<f64>,
    #[serde(default)]
    pub upper24_price: Option<f64>,
    #[serde(default)]
    pub rise_fall_rate: Option<f64>,
    #[serde(default)]
    pub index_price: Option<f64>,
    #[serde(default)]
    pub fair_price: Option<f64>,
    #[serde(default)]
    pub funding_rate: Option<f64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

impl MexcFuturesWsTicker {
    pub fn into_mark_price(self) -> Option<MarkPrice> {
        let symbol = mexc_futures_to_unified(&self.symbol);
        Some(MarkPrice {
            exchange: EXCHANGE,
            symbol,
            mark_price: Decimal::try_from(self.fair_price?).ok()?,
            index_price: self
                .index_price
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or_default(),
            timestamp_ms: self.timestamp.unwrap_or(0),
        })
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
    fn test_unified_to_mexc_futures() {
        let sym = Symbol::new("BTC", "USDT");
        assert_eq!(unified_to_mexc_futures(&sym), "BTC_USDT");

        let sym2 = Symbol::new("ETH", "USDT");
        assert_eq!(unified_to_mexc_futures(&sym2), "ETH_USDT");
    }

    #[test]
    fn test_mexc_futures_to_unified() {
        let u = mexc_futures_to_unified("BTC_USDT");
        assert_eq!(u, Symbol::new("BTC", "USDT"));

        let u2 = mexc_futures_to_unified("ETH_USDT");
        assert_eq!(u2, Symbol::new("ETH", "USDT"));
    }

    #[test]
    fn test_mexc_futures_to_unified_no_separator() {
        let u = mexc_futures_to_unified("BTCUSDT");
        assert_eq!(u.base, "BTCUSDT");
        assert_eq!(u.quote, "");
    }

    #[test]
    fn test_interval_mapping() {
        assert_eq!(interval_to_mexc_futures(Interval::M1), "Min1");
        assert_eq!(interval_to_mexc_futures(Interval::M5), "Min5");
        assert_eq!(interval_to_mexc_futures(Interval::M15), "Min15");
        assert_eq!(interval_to_mexc_futures(Interval::M30), "Min30");
        assert_eq!(interval_to_mexc_futures(Interval::H1), "Min60");
        assert_eq!(interval_to_mexc_futures(Interval::H4), "Hour4");
        assert_eq!(interval_to_mexc_futures(Interval::D1), "Day1");
        assert_eq!(interval_to_mexc_futures(Interval::W1), "Week1");
    }

    #[test]
    fn test_depth_conversion() {
        let raw = MexcFuturesDepthRaw {
            asks: vec![MexcFuturesDepthLevel(50001.0, 2.0, 5.0)],
            bids: vec![
                MexcFuturesDepthLevel(50000.0, 1.0, 3.0),
                MexcFuturesDepthLevel(49999.0, 0.5, 2.0),
            ],
            version: Some(42),
            timestamp: Some(1700000000000),
        };
        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"));
        assert_eq!(ob.exchange, ExchangeId::MexcFutures);
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.asks.len(), 1);
        assert_eq!(ob.bids[0].price, dec!(50000));
        assert_eq!(ob.asks[0].price, dec!(50001));
        assert_eq!(ob.sequence, Some(42));
        assert_eq!(ob.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_deal_conversion() {
        let raw = MexcFuturesDealRaw {
            p: 50000.5,
            v: 0.1,
            t_type: 1,
            t: 1700000000000,
            o: Some(1),
        };
        let trade = raw.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::MexcFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000.5));
        assert_eq!(trade.timestamp_ms, 1700000000000);
    }

    #[test]
    fn test_deal_sell() {
        let raw = MexcFuturesDealRaw {
            p: 2000.0,
            v: 1.0,
            t_type: 2,
            t: 1700000000000,
            o: None,
        };
        let trade = raw.into_trade(Symbol::new("ETH", "USDT"));
        assert_eq!(trade.side, Side::Sell);
    }

    #[test]
    fn test_ticker_conversion() {
        let raw = MexcFuturesTickerRaw {
            symbol: "BTC_USDT".into(),
            last_price: Some(50000.0),
            bid1: Some(49999.0),
            ask1: Some(50001.0),
            volume24: Some(12345.678),
            amount24: None,
            hold_vol: Some(100000.0),
            lower24_price: None,
            upper24_price: None,
            rise_fall_rate: Some(0.025),
            rise_fall_value: None,
            index_price: Some(50005.0),
            fair_price: Some(50010.0),
            funding_rate: Some(0.0001),
            max_bid_price: None,
            min_ask_price: None,
            timestamp: Some(1700000000000),
        };
        let ticker = raw.into_ticker();
        assert_eq!(ticker.exchange, ExchangeId::MexcFutures);
        assert_eq!(ticker.symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(ticker.last_price, dec!(50000));
        assert_eq!(ticker.bid, Some(dec!(49999)));
        assert_eq!(ticker.ask, Some(dec!(50001)));
    }

    #[test]
    fn test_ticker_to_funding_rate() {
        let raw = MexcFuturesTickerRaw {
            symbol: "BTC_USDT".into(),
            last_price: None,
            bid1: None,
            ask1: None,
            volume24: None,
            amount24: None,
            hold_vol: None,
            lower24_price: None,
            upper24_price: None,
            rise_fall_rate: None,
            rise_fall_value: None,
            index_price: None,
            fair_price: None,
            funding_rate: Some(0.0001),
            max_bid_price: None,
            min_ask_price: None,
            timestamp: Some(1700000000000),
        };
        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::MexcFutures);
        assert_eq!(fr.rate, dec!(0.0001));
    }

    #[test]
    fn test_ticker_to_mark_price() {
        let raw = MexcFuturesTickerRaw {
            symbol: "ETH_USDT".into(),
            last_price: None,
            bid1: None,
            ask1: None,
            volume24: None,
            amount24: None,
            hold_vol: None,
            lower24_price: None,
            upper24_price: None,
            rise_fall_rate: None,
            rise_fall_value: None,
            index_price: Some(2000.75),
            fair_price: Some(2001.5),
            funding_rate: None,
            max_bid_price: None,
            min_ask_price: None,
            timestamp: Some(1700000000000),
        };
        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::MexcFutures);
        assert_eq!(mp.mark_price, dec!(2001.5));
        assert_eq!(mp.index_price, dec!(2000.75));
    }

    #[test]
    fn test_ticker_to_open_interest() {
        let raw = MexcFuturesTickerRaw {
            symbol: "BTC_USDT".into(),
            last_price: None,
            bid1: None,
            ask1: None,
            volume24: None,
            amount24: None,
            hold_vol: Some(1000000.0),
            lower24_price: None,
            upper24_price: None,
            rise_fall_rate: None,
            rise_fall_value: None,
            index_price: None,
            fair_price: None,
            funding_rate: None,
            max_bid_price: None,
            min_ask_price: None,
            timestamp: Some(1700000000000),
        };
        let oi = raw.into_open_interest();
        assert_eq!(oi.exchange, ExchangeId::MexcFutures);
        assert_eq!(oi.open_interest, dec!(1000000));
    }

    #[test]
    fn test_kline_conversion() {
        let raw = MexcFuturesKlineRaw {
            time: Some(1700000000),
            open: Some(50000.0),
            close: Some(50100.0),
            high: Some(50200.0),
            low: Some(49900.0),
            vol: Some(500.0),
            amount: Some(25000000.0),
        };
        let candle = raw.into_candle(Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::MexcFutures);
        assert_eq!(candle.open, dec!(50000));
        assert_eq!(candle.close, dec!(50100));
        assert_eq!(candle.high, dec!(50200));
        assert_eq!(candle.low, dec!(49900));
        assert_eq!(candle.volume, dec!(500));
        assert_eq!(candle.open_time_ms, 1700000000000);
    }

    #[test]
    fn test_funding_rate_raw_conversion() {
        let raw = MexcFuturesFundingRateRaw {
            symbol: "BTC_USDT".into(),
            funding_rate: Some(0.0001),
            max_funding_rate: Some(0.003),
            min_funding_rate: Some(-0.003),
            collect_cycle: Some(28800),
            next_settle_time: Some(1700006400000),
            timestamp: Some(1700000000000),
        };
        let fr = raw.into_funding_rate();
        assert_eq!(fr.exchange, ExchangeId::MexcFutures);
        assert_eq!(fr.rate, dec!(0.0001));
        assert_eq!(fr.next_funding_time_ms, 1700006400000);
    }

    #[test]
    fn test_fair_price_conversion() {
        let raw = MexcFuturesFairPriceRaw {
            symbol: "ETH_USDT".into(),
            fair_price: Some(2001.5),
            index_price: Some(2000.75),
            timestamp: Some(1700000000000),
        };
        let mp = raw.into_mark_price();
        assert_eq!(mp.exchange, ExchangeId::MexcFutures);
        assert_eq!(mp.mark_price, dec!(2001.5));
        assert_eq!(mp.index_price, dec!(2000.75));
    }

    #[test]
    fn test_open_interest_raw_conversion() {
        let raw = MexcFuturesOpenInterestRaw {
            symbol: "BTC_USDT".into(),
            hold_vol: Some(50000.0),
            hold_coin: Some(1.0),
            hold_amount: Some(2500000000.0),
            timestamp: Some(1700000000000),
        };
        let oi = raw.into_open_interest();
        assert_eq!(oi.exchange, ExchangeId::MexcFutures);
        assert_eq!(oi.open_interest, dec!(50000));
        assert_eq!(oi.open_interest_value, dec!(2500000000));
    }

    #[test]
    fn test_ws_depth_full_conversion() {
        let raw = MexcFuturesWsDepthFull {
            asks: vec![MexcFuturesDepthLevel(50001.0, 2.0, 5.0)],
            bids: vec![MexcFuturesDepthLevel(50000.0, 1.0, 3.0)],
            version: Some(100),
        };
        let ob = raw.into_orderbook(Symbol::new("BTC", "USDT"), 1700000000000);
        assert_eq!(ob.exchange, ExchangeId::MexcFutures);
        assert_eq!(ob.bids[0].price, dec!(50000));
        assert_eq!(ob.asks[0].price, dec!(50001));
        assert_eq!(ob.sequence, Some(100));
    }

    #[test]
    fn test_ws_deal_conversion() {
        let raw = MexcFuturesWsDeal {
            p: 50000.0,
            v: 0.1,
            t_type: 1,
            t: 1700000000000,
            o: None,
            m: None,
        };
        let trade = raw.into_trade(Symbol::new("BTC", "USDT"));
        assert_eq!(trade.exchange, ExchangeId::MexcFutures);
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(50000));
    }

    #[test]
    fn test_ws_kline_conversion() {
        let raw = MexcFuturesWsKline {
            t: Some(1700000000),
            o: Some(50000.0),
            c: Some(50100.0),
            h: Some(50200.0),
            l: Some(49900.0),
            q: Some(500.0),
        };
        let candle = raw.into_candle(Symbol::new("BTC", "USDT")).unwrap();
        assert_eq!(candle.exchange, ExchangeId::MexcFutures);
        assert_eq!(candle.open, dec!(50000));
        assert_eq!(candle.open_time_ms, 1700000000000);
    }

    #[test]
    fn test_ws_ticker_to_mark_price() {
        let raw = MexcFuturesWsTicker {
            symbol: "BTC_USDT".into(),
            last_price: Some(50000.0),
            bid1: None,
            ask1: None,
            volume24: None,
            hold_vol: None,
            lower24_price: None,
            upper24_price: None,
            rise_fall_rate: None,
            index_price: Some(50005.0),
            fair_price: Some(50010.0),
            funding_rate: None,
            timestamp: Some(1700000000000),
        };
        let mp = raw.into_mark_price().unwrap();
        assert_eq!(mp.exchange, ExchangeId::MexcFutures);
        assert_eq!(mp.mark_price, dec!(50010));
        assert_eq!(mp.index_price, dec!(50005));
    }

    #[test]
    fn test_contracts_to_exchange_info() {
        let contracts = vec![MexcContractDetailRaw {
            symbol: "BTC_USDT".into(),
            display_name: Some("BTC_USDT".into()),
            base_coin: Some("BTC".into()),
            quote_coin: Some("USDT".into()),
            settle_coin: Some("USDT".into()),
            contract_size: Some(0.0001),
            min_leverage: Some(1.0),
            max_leverage: Some(200.0),
            price_scale: Some(1),
            vol_scale: Some(0),
            amount_scale: Some(4),
            price_unit: Some(0.1),
            vol_unit: Some(1.0),
            min_vol: Some(1.0),
            max_vol: Some(1000000.0),
            state: Some(0),
            is_new: Some(false),
            is_hot: Some(true),
            is_hidden: Some(false),
        }];
        let info = contracts_to_exchange_info(contracts);
        assert_eq!(info.exchange, ExchangeId::MexcFutures);
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].symbol, Symbol::new("BTC", "USDT"));
        assert_eq!(info.symbols[0].raw_symbol, "BTC_USDT");
        assert_eq!(info.symbols[0].status, SymbolStatus::Trading);
        assert_eq!(info.symbols[0].min_qty, Some(dec!(1)));
    }
}
