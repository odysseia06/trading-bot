use common::ExponentialBackoff;
use connector_core::{ConnectorConfig, ConnectorError, EventSender};
use futures_util::StreamExt;
use metrics::SharedMetrics;
use model::MarketEvent;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::parser::{parse_message, ParsedMessage};

const BINANCE_WS_BASE: &str = "wss://stream.binance.com:9443";

/// Duration of stable connection before resetting backoff.
const STABLE_CONNECTION_THRESHOLD: Duration = Duration::from_secs(300); // 5 minutes

fn build_stream_url(symbols: &[String]) -> String {
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@trade", s.to_lowercase()))
        .collect();

    if streams.len() == 1 {
        format!("{}/ws/{}", BINANCE_WS_BASE, streams[0])
    } else {
        format!("{}/stream?streams={}", BINANCE_WS_BASE, streams.join("/"))
    }
}

/// Run a single WebSocket connection session.
/// Returns Ok(()) if shutdown was requested, Err otherwise.
async fn run_session(
    url: &str,
    sender: &EventSender,
    shutdown_rx: &mut watch::Receiver<bool>,
    metrics: &SharedMetrics,
) -> Result<(), ConnectorError> {
    info!(url = %url, "Connecting to Binance WebSocket");

    let (ws_stream, _response) = connect_async(url)
        .await
        .map_err(|e| ConnectorError::WebSocket(e.to_string()))?;

    info!("Connected to Binance WebSocket");

    let (_write, mut read) = ws_stream.split();

    loop {
        tokio::select! {
            biased;

            // Check for shutdown signal
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Shutdown signal received, closing connection");
                    return Ok(());
                }
            }

            // Read from WebSocket
            msg_opt = read.next() => {
                let msg = match msg_opt {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket error");
                        metrics.inc_websocket_errors();
                        return Err(ConnectorError::WebSocket(e.to_string()));
                    }
                    None => {
                        info!("WebSocket stream ended");
                        return Err(ConnectorError::ConnectionClosed);
                    }
                };

                match msg {
                    Message::Text(text) => {
                        metrics.inc_messages_received();
                        match parse_message(&text) {
                            Ok(ParsedMessage::Trade(trade)) => {
                                metrics.inc_trades_received();
                                if sender.send(MarketEvent::Trade(trade)).await.is_err() {
                                    info!("Receiver dropped, stopping connector");
                                    return Err(ConnectorError::ChannelClosed);
                                }
                            }
                            Ok(ParsedMessage::Unknown) => {
                                // Ignore unknown messages silently
                            }
                            Err(e) => {
                                metrics.inc_parse_errors();
                                warn!(error = %e, "Failed to parse message");
                            }
                        }
                    }
                    Message::Ping(_) => {
                        // tungstenite handles pong automatically
                    }
                    Message::Close(_) => {
                        info!("WebSocket closed by server");
                        return Err(ConnectorError::ConnectionClosed);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Run the connector with automatic reconnection.
///
/// This function runs indefinitely, reconnecting on errors with exponential backoff.
/// It only returns when:
/// - The receiver channel is dropped (ChannelClosed)
/// - A shutdown signal is received via the shutdown_rx channel
pub async fn run_connector(
    config: ConnectorConfig,
    sender: EventSender,
    mut shutdown_rx: watch::Receiver<bool>,
    metrics: SharedMetrics,
) -> Result<(), ConnectorError> {
    let url = build_stream_url(&config.symbols);
    let mut backoff = ExponentialBackoff::default();

    loop {
        // Check if shutdown was requested before attempting connection
        if *shutdown_rx.borrow() {
            info!("Shutdown requested, exiting connector");
            return Ok(());
        }

        let session_start = std::time::Instant::now();

        match run_session(&url, &sender, &mut shutdown_rx, &metrics).await {
            Ok(()) => {
                // Clean shutdown requested
                info!("Connector shutdown complete");
                return Ok(());
            }
            Err(ConnectorError::ChannelClosed) => {
                // Receiver dropped, no point reconnecting
                info!("Channel closed, exiting connector");
                return Err(ConnectorError::ChannelClosed);
            }
            Err(e) => {
                // Connection error, attempt reconnect
                metrics.inc_reconnect_attempts();
                let session_duration = session_start.elapsed();

                // Reset backoff if connection was stable for a while
                if session_duration >= STABLE_CONNECTION_THRESHOLD {
                    info!(
                        duration_secs = session_duration.as_secs(),
                        "Connection was stable, resetting backoff"
                    );
                    backoff.reset();
                }

                let delay = backoff.next_delay();
                warn!(
                    error = %e,
                    attempt = backoff.attempt(),
                    delay_secs = delay.as_secs_f64(),
                    "Connection failed, reconnecting"
                );

                // Wait with shutdown check
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {
                        metrics.inc_reconnect_successes();
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Shutdown requested during backoff");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_stream_url_single() {
        let symbols = vec!["BTCUSDT".to_string()];
        let url = build_stream_url(&symbols);
        assert_eq!(url, "wss://stream.binance.com:9443/ws/btcusdt@trade");
    }

    #[test]
    fn test_build_stream_url_multiple() {
        let symbols = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let url = build_stream_url(&symbols);
        assert_eq!(
            url,
            "wss://stream.binance.com:9443/stream?streams=btcusdt@trade/ethusdt@trade"
        );
    }
}
