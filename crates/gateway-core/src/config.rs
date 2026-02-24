use std::time::Duration;

#[derive(Debug, Clone)]
pub struct WsConfig {
    pub reconnect_delay: Duration,
    pub max_reconnect_attempts: Option<u32>,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    pub orderbook_resync_on_gap: bool,
    /// Optional HTTP CONNECT proxy. Format: `host:port:user:pass` or `host:port`.
    pub proxy: Option<String>,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            reconnect_delay: Duration::from_secs(1),
            max_reconnect_attempts: None,
            ping_interval: Duration::from_secs(15),
            pong_timeout: Duration::from_secs(10),
            orderbook_resync_on_gap: true,
            proxy: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RestConfig {
    pub timeout: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_retries: 3,
            retry_delay: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExchangeConfig {
    pub rest: RestConfig,
    pub ws: WsConfig,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub passphrase: Option<String>,
}
