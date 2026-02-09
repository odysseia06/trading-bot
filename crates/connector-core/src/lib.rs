use common::BinanceEnvironment;
use model::MarketEvent;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Channel closed")]
    ChannelClosed,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Shutdown requested")]
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    /// Symbols to subscribe to.
    pub symbols: Vec<String>,
    /// Channel buffer capacity.
    pub channel_capacity: usize,
    /// Binance environment (production or testnet).
    pub environment: BinanceEnvironment,
    /// Enable order book depth stream (default: false).
    pub enable_depth_stream: bool,
    /// Depth update speed in milliseconds (100 or 1000, default: 100).
    pub depth_update_speed_ms: u32,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            symbols: vec!["BTCUSDT".to_string()],
            channel_capacity: 1024,
            environment: BinanceEnvironment::default(),
            enable_depth_stream: false,
            depth_update_speed_ms: 100,
        }
    }
}

pub type EventSender = mpsc::Sender<MarketEvent>;
pub type EventReceiver = mpsc::Receiver<MarketEvent>;

pub fn create_event_channel(capacity: usize) -> (EventSender, EventReceiver) {
    mpsc::channel(capacity)
}
