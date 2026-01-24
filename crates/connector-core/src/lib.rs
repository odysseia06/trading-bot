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
    pub symbols: Vec<String>,
    pub channel_capacity: usize,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            symbols: vec!["BTCUSDT".to_string()],
            channel_capacity: 1024,
        }
    }
}

pub type EventSender = mpsc::Sender<MarketEvent>;
pub type EventReceiver = mpsc::Receiver<MarketEvent>;

pub fn create_event_channel(capacity: usize) -> (EventSender, EventReceiver) {
    mpsc::channel(capacity)
}
