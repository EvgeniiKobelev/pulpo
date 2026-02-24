use crate::spot::mapper::*;
use crate::spot::proto::*;
use futures::{SinkExt, StreamExt};
use gateway_core::*;
use prost::Message as _;
use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://wbs-api.mexc.com/ws";
const EXCHANGE: ExchangeId = ExchangeId::Mexc;

// ---------------------------------------------------------------------------
// Core helpers
// ---------------------------------------------------------------------------

fn make_sub(channels: &[String]) -> String {
    serde_json::json!({
        "method": "SUBSCRIPTION",
        "params": channels
    })
    .to_string()
}

fn make_ping() -> String {
    serde_json::json!({ "method": "PING" }).to_string()
}

/// Decoded protobuf wrapper with its body variant.
#[allow(dead_code)]
enum DecodedMsg {
    Deals {
        symbol: String,
        create_time: i64,
        deals: PublicAggreDealsV3Api,
    },
    Depths {
        symbol: String,
        create_time: i64,
        depths: PublicAggreDepthsV3Api,
    },
    Kline {
        symbol: String,
        kline: PublicSpotKlineV3Api,
    },
    BookTicker {
        symbol: String,
        create_time: i64,
        ticker: PublicAggreBookTickerV3Api,
    },
    Unknown,
}

fn decode_push(data: &[u8]) -> DecodedMsg {
    let wrapper = match PushDataV3ApiWrapper::decode(data) {
        Ok(w) => w,
        Err(e) => {
            warn!("MEXC protobuf decode error: {e}");
            return DecodedMsg::Unknown;
        }
    };

    let symbol = wrapper.symbol.unwrap_or_default();
    let create_time = wrapper.create_time.unwrap_or(0);

    match wrapper.body {
        Some(WrapperBody::AggreDeals(deals)) => DecodedMsg::Deals {
            symbol,
            create_time,
            deals,
        },
        Some(WrapperBody::AggreDepths(depths)) => DecodedMsg::Depths {
            symbol,
            create_time,
            depths,
        },
        Some(WrapperBody::Kline(kline)) => DecodedMsg::Kline { symbol, kline },
        Some(WrapperBody::AggreBookTicker(ticker)) => DecodedMsg::BookTicker {
            symbol,
            create_time,
            ticker,
        },
        _ => DecodedMsg::Unknown,
    }
}

// ---------------------------------------------------------------------------
// WebSocket connection loop
// ---------------------------------------------------------------------------

async fn run_ws_loop(
    channels: Vec<String>,
    tx: mpsc::Sender<DecodedMsg>,
) {
    let mut backoff = Duration::from_secs(1);

    loop {
        if tx.is_closed() {
            break;
        }

        let ws = match tokio_tungstenite::connect_async(WS_URL).await {
            Ok((ws, _)) => {
                backoff = Duration::from_secs(1);
                info!("MEXC WS connected to {WS_URL}");
                ws
            }
            Err(e) => {
                warn!("MEXC WS connect failed: {e}, retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };

        let (mut write, mut read) = ws.split();

        // Subscribe
        let sub_msg = make_sub(&channels);
        if write.send(Message::text(&sub_msg)).await.is_err() {
            warn!("MEXC WS subscribe failed");
            continue;
        }

        let mut ping_interval = tokio::time::interval(Duration::from_secs(8));
        ping_interval.tick().await;

        loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    if write.send(Message::text(make_ping())).await.is_err() {
                        warn!("MEXC WS ping failed");
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Binary(data))) => {
                            let decoded = decode_push(&data);
                            if !matches!(decoded, DecodedMsg::Unknown) {
                                if tx.send(decoded).await.is_err() {
                                    debug!("MEXC WS receiver dropped");
                                    return;
                                }
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            // Control messages (sub confirmation, pong) come as text
                            if text.contains("Blocked") {
                                warn!("MEXC WS blocked: {text}");
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            let _ = write.send(Message::Pong(data)).await;
                        }
                        Some(Ok(Message::Close(_))) => {
                            warn!("MEXC WS closed by server");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("MEXC WS error: {e}");
                            break;
                        }
                        None => {
                            warn!("MEXC WS stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        if tx.is_closed() {
            break;
        }
        warn!("MEXC WS reconnecting in {backoff:?}");
        tokio::time::sleep(backoff).await;
    }
    debug!("MEXC WS loop ended");
}

/// Spawn a WS connection and return a stream of decoded protobuf messages.
fn subscribe_and_stream(
    channels: Vec<String>,
) -> mpsc::Receiver<DecodedMsg> {
    let (tx, rx) = mpsc::channel::<DecodedMsg>(1024);
    tokio::spawn(async move {
        run_ws_loop(channels, tx).await;
    });
    rx
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn pb_deal_to_trade(item: &PublicAggreDealsV3ApiItem, symbol: Symbol) -> Trade {
    let side = if item.trade_type == 1 {
        Side::Buy
    } else {
        Side::Sell
    };
    Trade {
        exchange: EXCHANGE,
        symbol,
        price: Decimal::from_str(&item.price).unwrap_or_default(),
        qty: Decimal::from_str(&item.quantity).unwrap_or_default(),
        side,
        timestamp_ms: item.time as u64,
        trade_id: None,
    }
}

fn pb_depths_to_orderbook(
    depths: &PublicAggreDepthsV3Api,
    symbol: Symbol,
    timestamp_ms: u64,
) -> OrderBook {
    let seq = depths.to_version.parse::<u64>().ok();
    OrderBook {
        exchange: EXCHANGE,
        symbol,
        bids: depths
            .bids
            .iter()
            .filter_map(|l| {
                let price = Decimal::from_str(&l.price).ok()?;
                let qty = Decimal::from_str(&l.quantity).ok()?;
                Some(Level::new(price, qty))
            })
            .collect(),
        asks: depths
            .asks
            .iter()
            .filter_map(|l| {
                let price = Decimal::from_str(&l.price).ok()?;
                let qty = Decimal::from_str(&l.quantity).ok()?;
                Some(Level::new(price, qty))
            })
            .collect(),
        timestamp_ms,
        sequence: seq,
    }
}

fn pb_kline_to_candle(kline: &PublicSpotKlineV3Api, symbol: Symbol) -> Option<Candle> {
    Some(Candle {
        exchange: EXCHANGE,
        symbol,
        open: Decimal::from_str(&kline.opening_price).ok()?,
        high: Decimal::from_str(&kline.highest_price).ok()?,
        low: Decimal::from_str(&kline.lowest_price).ok()?,
        close: Decimal::from_str(&kline.closing_price).ok()?,
        volume: Decimal::from_str(&kline.volume).ok().unwrap_or_default(),
        open_time_ms: kline.window_start as u64 * 1000,
        close_time_ms: kline.window_end as u64 * 1000,
        is_closed: false,
    })
}

// ---------------------------------------------------------------------------
// Single-symbol streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbook(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<OrderBook>> {
    let pair = unified_to_mexc(symbol);
    let channel = format!("spot@public.aggre.depth.v3.api.pb@100ms@{pair}");
    let sym = symbol.clone();
    let mut rx = subscribe_and_stream(vec![channel]);

    let (tx_out, rx_out) = mpsc::channel::<OrderBook>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let DecodedMsg::Depths {
                create_time,
                depths,
                ..
            } = msg
            {
                let ob = pb_depths_to_orderbook(&depths, sym.clone(), create_time as u64);
                if tx_out.send(ob).await.is_err() {
                    break;
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

pub async fn stream_trades(
    _config: &ExchangeConfig,
    symbol: &Symbol,
) -> Result<BoxStream<Trade>> {
    let pair = unified_to_mexc(symbol);
    let channel = format!("spot@public.aggre.deals.v3.api.pb@100ms@{pair}");
    let sym = symbol.clone();
    let mut rx = subscribe_and_stream(vec![channel]);

    let (tx_out, rx_out) = mpsc::channel::<Trade>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let DecodedMsg::Deals { deals, .. } = msg {
                for item in &deals.deals {
                    let trade = pb_deal_to_trade(item, sym.clone());
                    if tx_out.send(trade).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

pub async fn stream_candles(
    _config: &ExchangeConfig,
    symbol: &Symbol,
    interval: Interval,
) -> Result<BoxStream<Candle>> {
    let pair = unified_to_mexc(symbol);
    let iv = interval_to_mexc_ws(interval);
    let channel = format!("spot@public.kline.v3.api.pb@{pair}@{iv}");
    let sym = symbol.clone();
    let mut rx = subscribe_and_stream(vec![channel]);

    let (tx_out, rx_out) = mpsc::channel::<Candle>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let DecodedMsg::Kline { kline, .. } = msg {
                if let Some(candle) = pb_kline_to_candle(&kline, sym.clone()) {
                    if tx_out.send(candle).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

// ---------------------------------------------------------------------------
// Batch streams
// ---------------------------------------------------------------------------

pub async fn stream_orderbooks_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<OrderBook>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }
    let channels: Vec<String> = symbols
        .iter()
        .map(|sym| format!("spot@public.aggre.depth.v3.api.pb@100ms@{}", unified_to_mexc(sym)))
        .collect();
    let mut rx = subscribe_and_stream(channels);

    let (tx_out, rx_out) = mpsc::channel::<OrderBook>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let DecodedMsg::Depths {
                symbol,
                create_time,
                depths,
            } = msg
            {
                let sym = mexc_to_unified(&symbol);
                let ob = pb_depths_to_orderbook(&depths, sym, create_time as u64);
                if tx_out.send(ob).await.is_err() {
                    break;
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}

pub async fn stream_trades_batch(
    _config: &ExchangeConfig,
    symbols: &[Symbol],
) -> Result<BoxStream<Trade>> {
    if symbols.is_empty() {
        return Ok(Box::pin(futures::stream::empty()));
    }
    let channels: Vec<String> = symbols
        .iter()
        .map(|sym| format!("spot@public.aggre.deals.v3.api.pb@100ms@{}", unified_to_mexc(sym)))
        .collect();
    let mut rx = subscribe_and_stream(channels);

    let (tx_out, rx_out) = mpsc::channel::<Trade>(256);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let DecodedMsg::Deals {
                symbol, deals, ..
            } = msg
            {
                let sym = mexc_to_unified(&symbol);
                for item in &deals.deals {
                    let trade = pb_deal_to_trade(item, sym.clone());
                    if tx_out.send(trade).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx_out)))
}
