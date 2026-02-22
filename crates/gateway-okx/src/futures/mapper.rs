pub use crate::spot::mapper::{
    interval_to_okx, interval_to_okx_ws, okx_inst_id_to_unified, parse_kline_row, parse_levels,
    unified_to_okx, unified_to_okx_swap, OkxResponse, OkxWsArg, OkxWsBookData, OkxWsCandleMsg,
    OkxWsTradeData,
};

use gateway_core::{
    ExchangeId, FundingRate, Liquidation, MarkPrice, OpenInterest, Side, Symbol, SymbolInfo,
    SymbolStatus, Ticker, Trade,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Instrument (SWAP)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxSwapInstrumentRaw {
    pub inst_type: String,
    pub inst_id: String,
    #[serde(default)]
    pub settle_ccy: String,
    pub tick_sz: String,
    pub lot_sz: String,
    pub min_sz: String,
    pub state: String,
    #[serde(default)]
    pub ct_val: String,
    #[serde(default)]
    pub ct_type: String,
}

impl OkxSwapInstrumentRaw {
    pub fn into_symbol_info(self, _exchange: ExchangeId) -> Option<SymbolInfo> {
        let symbol = okx_inst_id_to_unified(&self.inst_id)?;
        let base_precision = precision_from_size(&self.lot_sz);
        let quote_precision = precision_from_size(&self.tick_sz);
        let min_qty = Decimal::from_str(&self.min_sz).ok();
        let tick_size = Decimal::from_str(&self.tick_sz).ok();
        let status = match self.state.as_str() {
            "live" => SymbolStatus::Trading,
            "suspend" => SymbolStatus::Halted,
            "preopen" => SymbolStatus::PreTrading,
            _ => SymbolStatus::Unknown,
        };
        Some(SymbolInfo {
            symbol,
            raw_symbol: self.inst_id,
            status,
            base_precision,
            quote_precision,
            min_qty,
            min_notional: None,
            tick_size,
        })
    }
}

fn precision_from_size(s: &str) -> u8 {
    if let Some(dot_pos) = s.find('.') {
        let after_dot = &s[dot_pos + 1..];
        let trimmed = after_dot.trim_end_matches('0');
        if trimmed.is_empty() {
            0
        } else {
            trimmed.len() as u8
        }
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Ticker (SWAP) — includes mark price, funding rate, open interest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxSwapTickerRaw {
    pub inst_id: String,
    pub last: String,
    pub ask_px: String,
    pub bid_px: String,
    #[serde(default)]
    pub open24h: String,
    pub vol24h: String,
    #[serde(default)]
    pub vol_ccy24h: String,
    pub ts: String,
    #[serde(default)]
    pub high24h: String,
    #[serde(default)]
    pub low24h: String,
}

impl OkxSwapTickerRaw {
    pub fn into_ticker(self, exchange: ExchangeId) -> Ticker {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        let last = Decimal::from_str(&self.last).unwrap_or_default();
        let open = Decimal::from_str(&self.open24h).unwrap_or_default();
        let pct = if !open.is_zero() {
            Some((last - open) / open * Decimal::from(100))
        } else {
            None
        };
        Ticker {
            exchange,
            symbol,
            last_price: last,
            bid: Decimal::from_str(&self.bid_px).ok(),
            ask: Decimal::from_str(&self.ask_px).ok(),
            volume_24h: Decimal::from_str(&self.vol24h).unwrap_or_default(),
            price_change_pct_24h: pct,
            timestamp_ms: self.ts.parse().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Trade (SWAP) — same structure as spot
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxSwapTradeRaw {
    pub inst_id: String,
    pub trade_id: String,
    pub px: String,
    pub sz: String,
    pub side: String,
    pub ts: String,
}

impl OkxSwapTradeRaw {
    pub fn into_trade(self, exchange: ExchangeId) -> Trade {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        Trade {
            exchange,
            symbol,
            price: Decimal::from_str(&self.px).unwrap_or_default(),
            qty: Decimal::from_str(&self.sz).unwrap_or_default(),
            side: if self.side == "buy" {
                Side::Buy
            } else {
                Side::Sell
            },
            timestamp_ms: self.ts.parse().unwrap_or(0),
            trade_id: Some(self.trade_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Funding Rate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxFundingRateRaw {
    pub inst_id: String,
    pub funding_rate: String,
    pub funding_time: String,
    #[serde(default)]
    pub next_funding_rate: String,
    #[serde(default)]
    pub next_funding_time: String,
}

impl OkxFundingRateRaw {
    pub fn into_funding_rate(self, exchange: ExchangeId) -> FundingRate {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        FundingRate {
            exchange,
            symbol,
            rate: Decimal::from_str(&self.funding_rate).unwrap_or_default(),
            next_funding_time_ms: self.next_funding_time.parse().unwrap_or(0),
            timestamp_ms: self.funding_time.parse().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Mark Price
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxMarkPriceRaw {
    pub inst_id: String,
    pub mark_px: String,
    pub ts: String,
}

impl OkxMarkPriceRaw {
    pub fn into_mark_price(self, exchange: ExchangeId) -> MarkPrice {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        MarkPrice {
            exchange,
            symbol,
            mark_price: Decimal::from_str(&self.mark_px).unwrap_or_default(),
            index_price: Decimal::ZERO,
            timestamp_ms: self.ts.parse().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Open Interest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxOpenInterestRaw {
    pub inst_id: String,
    pub oi: String,
    #[serde(default)]
    pub oi_ccy: String,
    #[serde(default)]
    pub oi_usd: String,
    pub ts: String,
}

impl OkxOpenInterestRaw {
    pub fn into_open_interest(self, exchange: ExchangeId) -> OpenInterest {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        OpenInterest {
            exchange,
            symbol,
            open_interest: Decimal::from_str(&self.oi_ccy).unwrap_or_default(),
            open_interest_value: Decimal::from_str(&self.oi_usd).unwrap_or_default(),
            timestamp_ms: self.ts.parse().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Liquidation Orders
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxLiquidationOrderRaw {
    pub inst_id: String,
    #[serde(default)]
    pub details: Vec<OkxLiquidationDetail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxLiquidationDetail {
    pub side: String,
    pub sz: String,
    pub px: String,
    pub ts: String,
}

impl OkxLiquidationOrderRaw {
    pub fn into_liquidations(self, exchange: ExchangeId) -> Vec<Liquidation> {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        self.details
            .into_iter()
            .map(|d| Liquidation {
                exchange,
                symbol: symbol.clone(),
                side: if d.side == "buy" {
                    Side::Buy
                } else {
                    Side::Sell
                },
                price: Decimal::from_str(&d.px).unwrap_or_default(),
                qty: Decimal::from_str(&d.sz).unwrap_or_default(),
                timestamp_ms: d.ts.parse().unwrap_or(0),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// WebSocket raw types — Mark Price
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxWsMarkPriceData {
    pub inst_id: String,
    pub mark_px: String,
    pub ts: String,
}

impl OkxWsMarkPriceData {
    pub fn into_mark_price(self, exchange: ExchangeId) -> MarkPrice {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        MarkPrice {
            exchange,
            symbol,
            mark_price: Decimal::from_str(&self.mark_px).unwrap_or_default(),
            index_price: Decimal::ZERO,
            timestamp_ms: self.ts.parse().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket raw types — Liquidation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxWsLiquidationData {
    pub inst_id: String,
    #[serde(default)]
    pub details: Vec<OkxLiquidationDetail>,
}

impl OkxWsLiquidationData {
    pub fn into_liquidations(self, exchange: ExchangeId) -> Vec<Liquidation> {
        let symbol = okx_inst_id_to_unified(&self.inst_id)
            .unwrap_or_else(|| Symbol::new("UNKNOWN", "UNKNOWN"));
        self.details
            .into_iter()
            .map(|d| Liquidation {
                exchange,
                symbol: symbol.clone(),
                side: if d.side == "buy" {
                    Side::Buy
                } else {
                    Side::Sell
                },
                price: Decimal::from_str(&d.px).unwrap_or_default(),
                qty: Decimal::from_str(&d.sz).unwrap_or_default(),
                timestamp_ms: d.ts.parse().unwrap_or(0),
            })
            .collect()
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
    fn test_swap_instrument_into_symbol_info() {
        let raw = OkxSwapInstrumentRaw {
            inst_type: "SWAP".into(),
            inst_id: "BTC-USDT-SWAP".into(),
            settle_ccy: "USDT".into(),
            tick_sz: "0.1".into(),
            lot_sz: "0.01".into(),
            min_sz: "0.01".into(),
            state: "live".into(),
            ct_val: "0.01".into(),
            ct_type: "linear".into(),
        };
        let info = raw.into_symbol_info(ExchangeId::Okx).unwrap();
        assert_eq!(info.symbol.base, "BTC");
        assert_eq!(info.symbol.quote, "USDT");
        assert_eq!(info.base_precision, 2);
        assert_eq!(info.quote_precision, 1);
        assert_eq!(info.status, SymbolStatus::Trading);
    }

    #[test]
    fn test_funding_rate_raw() {
        let raw = OkxFundingRateRaw {
            inst_id: "BTC-USDT-SWAP".into(),
            funding_rate: "0.0001".into(),
            funding_time: "1700000000000".into(),
            next_funding_rate: "0.00015".into(),
            next_funding_time: "1700028800000".into(),
        };
        let fr = raw.into_funding_rate(ExchangeId::Okx);
        assert_eq!(fr.symbol.base, "BTC");
        assert_eq!(fr.rate, dec!(0.0001));
        assert_eq!(fr.next_funding_time_ms, 1700028800000);
    }

    #[test]
    fn test_mark_price_raw() {
        let raw = OkxMarkPriceRaw {
            inst_id: "ETH-USDT-SWAP".into(),
            mark_px: "2100.55".into(),
            ts: "1700000000000".into(),
        };
        let mp = raw.into_mark_price(ExchangeId::Okx);
        assert_eq!(mp.symbol.base, "ETH");
        assert_eq!(mp.mark_price, dec!(2100.55));
        assert_eq!(mp.index_price, Decimal::ZERO);
    }

    #[test]
    fn test_open_interest_raw() {
        let raw = OkxOpenInterestRaw {
            inst_id: "BTC-USDT-SWAP".into(),
            oi: "14546007".into(),
            oi_ccy: "1454.6007".into(),
            oi_usd: "30921319.84".into(),
            ts: "1700000000000".into(),
        };
        let oi = raw.into_open_interest(ExchangeId::Okx);
        assert_eq!(oi.symbol.base, "BTC");
        assert_eq!(oi.open_interest, dec!(1454.6007));
        assert_eq!(oi.open_interest_value, dec!(30921319.84));
    }

    #[test]
    fn test_liquidation_raw() {
        let raw = OkxLiquidationOrderRaw {
            inst_id: "BTC-USDT-SWAP".into(),
            details: vec![OkxLiquidationDetail {
                side: "sell".into(),
                sz: "0.5".into(),
                px: "43000.0".into(),
                ts: "1700000000000".into(),
            }],
        };
        let liqs = raw.into_liquidations(ExchangeId::Okx);
        assert_eq!(liqs.len(), 1);
        assert_eq!(liqs[0].side, Side::Sell);
        assert_eq!(liqs[0].price, dec!(43000.0));
        assert_eq!(liqs[0].qty, dec!(0.5));
    }

    #[test]
    fn test_swap_trade_raw() {
        let raw = OkxSwapTradeRaw {
            inst_id: "BTC-USDT-SWAP".into(),
            trade_id: "555".into(),
            px: "44000.0".into(),
            sz: "1.0".into(),
            side: "buy".into(),
            ts: "1700000000000".into(),
        };
        let trade = raw.into_trade(ExchangeId::Okx);
        assert_eq!(trade.symbol.base, "BTC");
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.price, dec!(44000.0));
    }

    #[test]
    fn test_swap_ticker_raw() {
        let raw = OkxSwapTickerRaw {
            inst_id: "ETH-USDT-SWAP".into(),
            last: "2100.0".into(),
            ask_px: "2100.1".into(),
            bid_px: "2099.9".into(),
            open24h: "2000.0".into(),
            vol24h: "100000".into(),
            vol_ccy24h: "210000000".into(),
            ts: "1700000000000".into(),
            high24h: "2150.0".into(),
            low24h: "1980.0".into(),
        };
        let ticker = raw.into_ticker(ExchangeId::Okx);
        assert_eq!(ticker.symbol.base, "ETH");
        assert_eq!(ticker.last_price, dec!(2100.0));
        assert!(ticker.price_change_pct_24h.is_some());
    }

    #[test]
    fn test_ws_mark_price_data() {
        let data = OkxWsMarkPriceData {
            inst_id: "BTC-USDT-SWAP".into(),
            mark_px: "45000.5".into(),
            ts: "1700000000000".into(),
        };
        let mp = data.into_mark_price(ExchangeId::Okx);
        assert_eq!(mp.mark_price, dec!(45000.5));
    }

    #[test]
    fn test_ws_liquidation_data() {
        let data = OkxWsLiquidationData {
            inst_id: "BTC-USDT-SWAP".into(),
            details: vec![
                OkxLiquidationDetail {
                    side: "buy".into(),
                    sz: "0.1".into(),
                    px: "42000.0".into(),
                    ts: "1700000000000".into(),
                },
                OkxLiquidationDetail {
                    side: "sell".into(),
                    sz: "0.2".into(),
                    px: "43000.0".into(),
                    ts: "1700000000001".into(),
                },
            ],
        };
        let liqs = data.into_liquidations(ExchangeId::Okx);
        assert_eq!(liqs.len(), 2);
        assert_eq!(liqs[0].side, Side::Buy);
        assert_eq!(liqs[1].side, Side::Sell);
    }
}
