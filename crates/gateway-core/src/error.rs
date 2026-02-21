use thiserror::Error;
use crate::types::ExchangeId;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("[{exchange}] REST error: {message}")]
    Rest { exchange: ExchangeId, message: String, status: Option<u16> },

    #[error("[{exchange}] WebSocket error: {message}")]
    WebSocket { exchange: ExchangeId, message: String },

    #[error("[{exchange}] Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { exchange: ExchangeId, retry_after_ms: u64 },

    #[error("[{exchange}] Symbol not found: {symbol}")]
    SymbolNotFound { exchange: ExchangeId, symbol: String },

    #[error("[{exchange}] Auth error: {message}")]
    Auth { exchange: ExchangeId, message: String },

    #[error("[{exchange}] Parse error: {message}")]
    Parse { exchange: ExchangeId, message: String },

    #[error("Connection lost to {exchange}")]
    Disconnected { exchange: ExchangeId },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;
