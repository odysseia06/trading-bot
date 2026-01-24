use common::ExponentialBackoff;
use connector_core::{ConnectorConfig, ConnectorError, EventSender};
use futures_util::{SinkExt, StreamExt};
use metrics::SharedMetrics;
use model::MarketEvent;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::parser::{parse_message, ParsedMessage};

const BINANCE_WS_BASE: &str = "wss://stream.binance.com:9443";

/// Duration of stable connection before resetting backoff.
const STABLE_CONNECTION_THRESHOLD: Duration = Duration::from_secs(300); // 5 minutes

/// Timeout for WebSocket connection attempts.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Result of a connection attempt.
enum ConnectResult {
    Connected(
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ),
    Shutdown,
    Error(ConnectorError),
}

/// Attempt to connect with timeout and shutdown check.
async fn connect_with_timeout(url: &str, shutdown_rx: &mut watch::Receiver<bool>) -> ConnectResult {
    tokio::select! {
        biased;

        // Check for shutdown during connection
        _ = shutdown_rx.changed() => {
            if *shutdown_rx.borrow() {
                return ConnectResult::Shutdown;
            }
            // Spurious wakeup, fall through to connection attempt
        }

        // Connection with timeout
        result = tokio::time::timeout(CONNECTION_TIMEOUT, connect_async(url)) => {
            return match result {
                Ok(Ok((stream, _))) => ConnectResult::Connected(stream),
                Ok(Err(e)) => ConnectResult::Error(ConnectorError::WebSocket(e.to_string())),
                Err(_) => ConnectResult::Error(ConnectorError::WebSocket("connection timeout".to_string())),
            };
        }
    }

    // Handle spurious wakeup case - try connection with shutdown check
    tokio::select! {
        biased;

        _ = shutdown_rx.changed() => {
            if *shutdown_rx.borrow() {
                return ConnectResult::Shutdown;
            }
        }

        result = tokio::time::timeout(CONNECTION_TIMEOUT, connect_async(url)) => {
            return match result {
                Ok(Ok((stream, _))) => ConnectResult::Connected(stream),
                Ok(Err(e)) => ConnectResult::Error(ConnectorError::WebSocket(e.to_string())),
                Err(_) => ConnectResult::Error(ConnectorError::WebSocket("connection timeout".to_string())),
            };
        }
    }

    // If we get here, it's another spurious wakeup during the second select
    // Just return shutdown if requested, otherwise error
    if *shutdown_rx.borrow() {
        ConnectResult::Shutdown
    } else {
        ConnectResult::Error(ConnectorError::WebSocket(
            "connection interrupted".to_string(),
        ))
    }
}

/// Result of a session.
enum SessionResult {
    /// Session ran and then shutdown was requested
    Shutdown,
    /// Session connected successfully (returns session duration when it ended)
    Connected {
        duration: Duration,
        error: ConnectorError,
    },
    /// Failed to connect
    ConnectFailed(ConnectorError),
}

/// Run a single WebSocket connection session.
async fn run_session(
    url: &str,
    sender: &EventSender,
    shutdown_rx: &mut watch::Receiver<bool>,
    metrics: &SharedMetrics,
) -> SessionResult {
    info!(url = %url, "Connecting to Binance WebSocket");

    let ws_stream = match connect_with_timeout(url, shutdown_rx).await {
        ConnectResult::Connected(stream) => stream,
        ConnectResult::Shutdown => return SessionResult::Shutdown,
        ConnectResult::Error(e) => return SessionResult::ConnectFailed(e),
    };

    info!("Connected to Binance WebSocket");
    let connected_at = std::time::Instant::now();

    let (mut write, mut read) = ws_stream.split();

    loop {
        tokio::select! {
            biased;

            // Check for shutdown signal
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Shutdown signal received, closing connection");
                    // Try to send close frame gracefully
                    let _ = write.close().await;
                    return SessionResult::Shutdown;
                }
            }

            // Read from WebSocket
            msg_opt = read.next() => {
                let msg = match msg_opt {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket error");
                        metrics.inc_websocket_errors();
                        return SessionResult::Connected {
                            duration: connected_at.elapsed(),
                            error: ConnectorError::WebSocket(e.to_string()),
                        };
                    }
                    None => {
                        info!("WebSocket stream ended");
                        return SessionResult::Connected {
                            duration: connected_at.elapsed(),
                            error: ConnectorError::ConnectionClosed,
                        };
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
                                    return SessionResult::Connected {
                                        duration: connected_at.elapsed(),
                                        error: ConnectorError::ChannelClosed,
                                    };
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
                    Message::Ping(data) => {
                        // Respond with Pong to keep connection alive
                        debug!("Received Ping, sending Pong");
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            warn!(error = %e, "Failed to send Pong");
                            metrics.inc_websocket_errors();
                            return SessionResult::Connected {
                                duration: connected_at.elapsed(),
                                error: ConnectorError::WebSocket(e.to_string()),
                            };
                        }
                    }
                    Message::Close(_) => {
                        info!("WebSocket closed by server");
                        return SessionResult::Connected {
                            duration: connected_at.elapsed(),
                            error: ConnectorError::ConnectionClosed,
                        };
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
    let mut needs_reconnect = false; // True after we've connected and then disconnected

    loop {
        // Check if shutdown was requested before attempting connection
        if *shutdown_rx.borrow() {
            info!("Shutdown requested, exiting connector");
            return Ok(());
        }

        match run_session(&url, &sender, &mut shutdown_rx, &metrics).await {
            SessionResult::Shutdown => {
                // Clean shutdown requested - don't log as error or increment metrics
                info!("Connector shutdown complete");
                return Ok(());
            }
            SessionResult::Connected { duration, error } => {
                // We were connected, now we're not - this is a reconnect scenario
                if needs_reconnect {
                    // We successfully reconnected (connected after a previous disconnect)
                    metrics.inc_reconnect_successes();
                }

                // Now we need to reconnect
                needs_reconnect = true;

                // Handle channel closed specially - no point reconnecting
                if matches!(error, ConnectorError::ChannelClosed) {
                    info!("Channel closed, exiting connector");
                    return Err(ConnectorError::ChannelClosed);
                }

                // Reset backoff if connection was stable
                if duration >= STABLE_CONNECTION_THRESHOLD {
                    info!(
                        duration_secs = duration.as_secs(),
                        "Connection was stable, resetting backoff"
                    );
                    backoff.reset();
                }

                metrics.inc_reconnect_attempts();

                let delay = backoff.next_delay();
                warn!(
                    error = %error,
                    attempt = backoff.attempt(),
                    delay_secs = delay.as_secs_f64(),
                    "Connection lost, reconnecting"
                );

                // Wait with shutdown check
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Shutdown requested during backoff");
                            return Ok(());
                        }
                    }
                }
            }
            SessionResult::ConnectFailed(e) => {
                // Failed to connect at all
                if needs_reconnect {
                    metrics.inc_reconnect_attempts();
                }

                let delay = backoff.next_delay();
                warn!(
                    error = %e,
                    attempt = backoff.attempt(),
                    delay_secs = delay.as_secs_f64(),
                    "Connection failed, retrying"
                );

                // Wait with shutdown check
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
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
