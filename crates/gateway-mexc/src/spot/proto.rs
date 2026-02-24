//! MEXC WebSocket v3 protobuf message types.
//!
//! Manually defined from https://github.com/mexcdevelop/websocket-proto

/// Outer envelope for all MEXC WS push messages.
#[derive(Clone, prost::Message)]
pub struct PushDataV3ApiWrapper {
    /// Channel name, e.g. "spot@public.aggre.deals.v3.api.pb@100ms@BTCUSDT"
    #[prost(string, tag = "1")]
    pub channel: String,
    /// Trading pair, e.g. "BTCUSDT"
    #[prost(string, optional, tag = "3")]
    pub symbol: Option<String>,
    #[prost(string, optional, tag = "4")]
    pub symbol_id: Option<String>,
    /// Message creation time (ms)
    #[prost(int64, optional, tag = "5")]
    pub create_time: Option<i64>,
    /// Message push time (ms)
    #[prost(int64, optional, tag = "6")]
    pub send_time: Option<i64>,
    #[prost(oneof = "WrapperBody", tags = "308, 313, 314, 315")]
    pub body: Option<WrapperBody>,
}

#[derive(Clone, prost::Oneof)]
pub enum WrapperBody {
    #[prost(message, tag = "308")]
    Kline(PublicSpotKlineV3Api),
    #[prost(message, tag = "313")]
    AggreDepths(PublicAggreDepthsV3Api),
    #[prost(message, tag = "314")]
    AggreDeals(PublicAggreDealsV3Api),
    #[prost(message, tag = "315")]
    AggreBookTicker(PublicAggreBookTickerV3Api),
}

// --- Trades ---

#[derive(Clone, prost::Message)]
pub struct PublicAggreDealsV3Api {
    #[prost(message, repeated, tag = "1")]
    pub deals: Vec<PublicAggreDealsV3ApiItem>,
    #[prost(string, tag = "2")]
    pub event_type: String,
}

#[derive(Clone, prost::Message)]
pub struct PublicAggreDealsV3ApiItem {
    #[prost(string, tag = "1")]
    pub price: String,
    #[prost(string, tag = "2")]
    pub quantity: String,
    /// 1 = buy, 2 = sell
    #[prost(int32, tag = "3")]
    pub trade_type: i32,
    /// Timestamp in milliseconds
    #[prost(int64, tag = "4")]
    pub time: i64,
}

// --- Order book depth ---

#[derive(Clone, prost::Message)]
pub struct PublicAggreDepthsV3Api {
    #[prost(message, repeated, tag = "1")]
    pub asks: Vec<PublicAggreDepthV3ApiItem>,
    #[prost(message, repeated, tag = "2")]
    pub bids: Vec<PublicAggreDepthV3ApiItem>,
    #[prost(string, tag = "3")]
    pub event_type: String,
    #[prost(string, tag = "4")]
    pub from_version: String,
    #[prost(string, tag = "5")]
    pub to_version: String,
}

#[derive(Clone, prost::Message)]
pub struct PublicAggreDepthV3ApiItem {
    #[prost(string, tag = "1")]
    pub price: String,
    #[prost(string, tag = "2")]
    pub quantity: String,
}

// --- Book ticker (best bid/ask) ---

#[derive(Clone, prost::Message)]
pub struct PublicAggreBookTickerV3Api {
    #[prost(string, tag = "1")]
    pub bid_price: String,
    #[prost(string, tag = "2")]
    pub bid_quantity: String,
    #[prost(string, tag = "3")]
    pub ask_price: String,
    #[prost(string, tag = "4")]
    pub ask_quantity: String,
}

// --- Klines ---

#[derive(Clone, prost::Message)]
pub struct PublicSpotKlineV3Api {
    #[prost(string, tag = "1")]
    pub interval: String,
    /// Window start timestamp (seconds)
    #[prost(int64, tag = "2")]
    pub window_start: i64,
    #[prost(string, tag = "3")]
    pub opening_price: String,
    #[prost(string, tag = "4")]
    pub closing_price: String,
    #[prost(string, tag = "5")]
    pub highest_price: String,
    #[prost(string, tag = "6")]
    pub lowest_price: String,
    #[prost(string, tag = "7")]
    pub volume: String,
    #[prost(string, tag = "8")]
    pub amount: String,
    /// Window end timestamp (seconds)
    #[prost(int64, tag = "9")]
    pub window_end: i64,
}
