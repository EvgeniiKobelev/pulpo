use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeId {
    BinanceSpot,
    BinanceFutures,
    BitgetSpot,
    BitgetFutures,
    BybitSpot,
    BybitFutures,
    Okx,
    Gate,
    GateFutures,
    Hyperliquid,
    Kucoin,
    Mexc,
    MexcFutures,
    LighterFutures,
    AsterdexFutures,
    PhemexFutures,
}

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BinanceSpot => write!(f, "binance_spot"),
            Self::BinanceFutures => write!(f, "binance_futures"),
            Self::BitgetSpot => write!(f, "bitget_spot"),
            Self::BitgetFutures => write!(f, "bitget_futures"),
            Self::BybitSpot => write!(f, "bybit_spot"),
            Self::BybitFutures => write!(f, "bybit_futures"),
            Self::Okx => write!(f, "okx"),
            Self::Gate => write!(f, "gate"),
            Self::GateFutures => write!(f, "gate_futures"),
            Self::Hyperliquid => write!(f, "hyperliquid"),
            Self::Kucoin => write!(f, "kucoin"),
            Self::Mexc => write!(f, "mexc"),
            Self::MexcFutures => write!(f, "mexc_futures"),
            Self::LighterFutures => write!(f, "lighter_futures"),
            Self::AsterdexFutures => write!(f, "asterdex_futures"),
            Self::PhemexFutures => write!(f, "phemex_futures"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol {
    pub base: String,
    pub quote: String,
}

impl Symbol {
    pub fn new(base: impl Into<String>, quote: impl Into<String>) -> Self {
        Self {
            base: base.into().to_uppercase(),
            quote: quote.into().to_uppercase(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let (base, quote) = s.split_once('/')?;
        Some(Self::new(base.trim(), quote.trim()))
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.base, self.quote)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Level {
    pub price: Decimal,
    pub qty: Decimal,
}

impl Level {
    pub fn new(price: Decimal, qty: Decimal) -> Self {
        Self { price, qty }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub timestamp_ms: u64,
    pub sequence: Option<u64>,
}

impl OrderBook {
    pub fn best_bid(&self) -> Option<&Level> { self.bids.first() }
    pub fn best_ask(&self) -> Option<&Level> { self.asks.first() }
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_ask(), self.best_bid()) {
            (Some(ask), Some(bid)) => Some(ask.price - bid.price),
            _ => None,
        }
    }
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_ask(), self.best_bid()) {
            (Some(ask), Some(bid)) => Some((ask.price + bid.price) / Decimal::TWO),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side { Buy, Sell }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub price: Decimal,
    pub qty: Decimal,
    pub side: Side,
    pub timestamp_ms: u64,
    pub trade_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub open_time_ms: u64,
    pub close_time_ms: u64,
    pub is_closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticker {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub last_price: Decimal,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub volume_24h: Decimal,
    pub price_change_pct_24h: Option<Decimal>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interval { S1, M1, M3, M5, M15, M30, H1, H4, D1, W1 }

impl Interval {
    pub fn as_secs(&self) -> u64 {
        match self {
            Self::S1 => 1, Self::M1 => 60, Self::M3 => 180, Self::M5 => 300,
            Self::M15 => 900, Self::M30 => 1800, Self::H1 => 3600,
            Self::H4 => 14400, Self::D1 => 86400, Self::W1 => 604800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRate {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub rate: Decimal,
    pub next_funding_time_ms: u64,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkPrice {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub mark_price: Decimal,
    pub index_price: Decimal,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenInterest {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub open_interest: Decimal,
    pub open_interest_value: Decimal,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Liquidation {
    pub exchange: ExchangeId,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Decimal,
    pub qty: Decimal,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub symbol: Symbol,
    pub raw_symbol: String,
    pub status: SymbolStatus,
    pub base_precision: u8,
    pub quote_precision: u8,
    pub min_qty: Option<Decimal>,
    pub min_notional: Option<Decimal>,
    pub tick_size: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolStatus { Trading, Halted, PreTrading, Unknown }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeInfo {
    pub exchange: ExchangeId,
    pub symbols: Vec<SymbolInfo>,
}
