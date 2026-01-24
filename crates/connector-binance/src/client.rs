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

/// Attempt to connect with timeout and shutdown check.
async fn connect_with_timeout(
    url: &str,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ConnectorError,
> {
    tokio::select! {
        biased;

        // Check for shutdown during connection
        _ = shutdown_rx.changed() => {
            if *shutdown_rx.borrow() {
                return Err(ConnectorError::ConnectionClosed);
            }
            // Spurious wakeup, continue with connection
            match tokio::time::timeout(CONNECTION_TIMEOUT, connect_async(url)).await {
                Ok(Ok((stream, _))) => Ok(stream),
                Ok(Err(e)) => Err(ConnectorError::WebSocket(e.to_string())),
                Err(_) => Err(ConnectorError::WebSocket("connection timeout".to_string())),
            }
        }

        // Connection with timeout
        result = tokio::time::timeout(CONNECTION_TIMEOUT, connect_async(url)) => {
            match result {
                Ok(Ok((stream, _))) => Ok(stream),
                Ok(Err(e)) => Err(ConnectorError::WebSocket(e.to_string())),
                Err(_) => Err(ConnectorError::WebSocket("connection timeout".to_string())),
            }
        }
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

    let ws_stream = connect_with_timeout(url, shutdown_rx).await?;

    info!("Connected to Binance WebSocket");

    // Note: tokio-tungstenite with tungstenite automatically responds to Ping frames
    // with Pong frames at the protocol level. We don't need the write half for this
    // basic read-only use case. The library handles Ping/Pong internally before
    // surfacing messages to the application layer.
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
                        // tungstenite handles Pong response automatically at the protocol level
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
    let mut is_first_connect = true;

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
                // Only count as reconnect attempt if this wasn't the first connection
                if !is_first_connect {
                    metrics.inc_reconnect_attempts();
                }
                is_first_connect = false;

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
                        // Continue to next iteration to attempt reconnect
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

        // If we reach here after backoff, we're about to attempt reconnection
        // We'll count success only after run_session succeeds (tracked via stable connection)
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
