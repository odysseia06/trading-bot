use binance_rest::BinanceRestClient;
use common::{BinanceEnvironment, ExponentialBackoff};
use connector_core::{ConnectorConfig, ConnectorError, EventSender};
use futures_util::{SinkExt, StreamExt};
use metrics::SharedMetrics;
use model::{DepthUpdate, MarketEvent};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::depth_manager::{DepthManager, ProcessResult};
use crate::parser::{parse_message, ParsedMessage};

/// Duration of stable connection before resetting backoff.
const STABLE_CONNECTION_THRESHOLD: Duration = Duration::from_secs(300); // 5 minutes

/// Timeout for WebSocket connection attempts.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

fn build_stream_url(
    symbols: &[String],
    environment: BinanceEnvironment,
    enable_depth: bool,
    depth_speed_ms: u32,
) -> String {
    let base_url = environment.ws_base_url();
    let mut streams: Vec<String> = Vec::new();

    for symbol in symbols {
        let sym_lower = symbol.to_lowercase();
        // Always subscribe to trade stream
        streams.push(format!("{}@trade", sym_lower));
        // Optionally subscribe to depth stream
        if enable_depth {
            streams.push(format!("{}@depth@{}ms", sym_lower, depth_speed_ms));
        }
    }

    if streams.len() == 1 {
        format!("{}/ws/{}", base_url, streams[0])
    } else {
        format!("{}/stream?streams={}", base_url, streams.join("/"))
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

/// Context for depth stream synchronization.
struct DepthContext {
    manager: DepthManager,
    rest_client: Arc<BinanceRestClient>,
    pending_snapshots: HashSet<String>,
}

impl DepthContext {
    fn new(rest_client: Arc<BinanceRestClient>) -> Self {
        Self {
            manager: DepthManager::new(),
            rest_client,
            pending_snapshots: HashSet::new(),
        }
    }

    /// Process a depth update through the manager.
    /// Returns events to send (if any) and symbols needing snapshots.
    fn process_update(&mut self, update: DepthUpdate) -> (Vec<MarketEvent>, Option<String>) {
        match self.manager.process_update(update) {
            ProcessResult::Apply(depth) => (vec![MarketEvent::DepthUpdate(depth)], None),
            ProcessResult::Buffered => (vec![], None),
            ProcessResult::Dropped => (vec![], None),
            ProcessResult::NeedSnapshot(symbol) => {
                if !self.pending_snapshots.contains(&symbol) {
                    self.pending_snapshots.insert(symbol.clone());
                    (vec![], Some(symbol))
                } else {
                    (vec![], None)
                }
            }
        }
    }

    /// Fetch and apply a snapshot, returning events to send.
    async fn fetch_snapshot(&mut self, symbol: &str, metrics: &SharedMetrics) -> Vec<MarketEvent> {
        info!(symbol = %symbol, "Fetching depth snapshot");

        match self.rest_client.get_depth_snapshot(symbol, 1000).await {
            Ok(snapshot) => {
                metrics.inc_depth_snapshots_fetched();
                self.pending_snapshots.remove(symbol);

                let result = self.manager.apply_snapshot(
                    symbol,
                    snapshot.last_update_id,
                    &snapshot.bids,
                    &snapshot.asks,
                );

                // Convert events to MarketEvents
                result
                    .events_to_apply
                    .into_iter()
                    .map(MarketEvent::DepthUpdate)
                    .collect()
            }
            Err(e) => {
                warn!(symbol = %symbol, error = %e, "Failed to fetch depth snapshot");
                self.pending_snapshots.remove(symbol);
                // Clear the manager state to retry on next update
                self.manager.clear(symbol);
                vec![]
            }
        }
    }
}

/// Run a single WebSocket connection session.
async fn run_session(
    url: &str,
    sender: &EventSender,
    shutdown_rx: &mut watch::Receiver<bool>,
    metrics: &SharedMetrics,
    mut depth_ctx: Option<&mut DepthContext>,
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
                            Ok(ParsedMessage::DepthUpdate(depth)) => {
                                metrics.inc_depth_updates_received();

                                // Process through depth manager if available
                                if let Some(ref mut ctx) = depth_ctx {
                                    let (events, need_snapshot) = ctx.process_update(depth);

                                    // Send any events ready to be applied
                                    for event in events {
                                        if sender.send(event).await.is_err() {
                                            info!("Receiver dropped, stopping connector");
                                            return SessionResult::Connected {
                                                duration: connected_at.elapsed(),
                                                error: ConnectorError::ChannelClosed,
                                            };
                                        }
                                    }

                                    // Fetch snapshot if needed (in background to not block)
                                    if let Some(sym) = need_snapshot {
                                        let snapshot_events = ctx.fetch_snapshot(&sym, metrics).await;
                                        for event in snapshot_events {
                                            if sender.send(event).await.is_err() {
                                                info!("Receiver dropped, stopping connector");
                                                return SessionResult::Connected {
                                                    duration: connected_at.elapsed(),
                                                    error: ConnectorError::ChannelClosed,
                                                };
                                            }
                                        }
                                    }
                                } else {
                                    // No depth context - send directly (legacy behavior)
                                    if sender.send(MarketEvent::DepthUpdate(depth)).await.is_err() {
                                        info!("Receiver dropped, stopping connector");
                                        return SessionResult::Connected {
                                            duration: connected_at.elapsed(),
                                            error: ConnectorError::ChannelClosed,
                                        };
                                    }
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
///
/// # Arguments
/// * `config` - Connector configuration
/// * `sender` - Channel to send market events
/// * `shutdown_rx` - Shutdown signal receiver
/// * `metrics` - Shared metrics collector
/// * `rest_client` - Optional REST client for depth snapshot fetching
pub async fn run_connector(
    config: ConnectorConfig,
    sender: EventSender,
    mut shutdown_rx: watch::Receiver<bool>,
    metrics: SharedMetrics,
    rest_client: Option<Arc<BinanceRestClient>>,
) -> Result<(), ConnectorError> {
    let url = build_stream_url(
        &config.symbols,
        config.environment,
        config.enable_depth_stream,
        config.depth_update_speed_ms,
    );
    let mut backoff = ExponentialBackoff::default();
    let mut needs_reconnect = false; // True after we've had at least one connection attempt

    // Create depth context if depth stream is enabled and we have a REST client
    let mut depth_ctx = if config.enable_depth_stream {
        match rest_client {
            Some(client) => {
                info!("Depth stream enabled with synchronization");
                Some(DepthContext::new(client))
            }
            None => {
                warn!("Depth stream enabled but no REST client provided - running without synchronization");
                None
            }
        }
    } else {
        None
    };

    loop {
        // Check if shutdown was requested before attempting connection
        if *shutdown_rx.borrow() {
            info!("Shutdown requested, exiting connector");
            return Ok(());
        }

        match run_session(
            &url,
            &sender,
            &mut shutdown_rx,
            &metrics,
            depth_ctx.as_mut(),
        )
        .await
        {
            SessionResult::Shutdown => {
                // Clean shutdown requested - don't log as error or increment metrics
                info!("Connector shutdown complete");
                return Ok(());
            }
            SessionResult::Connected { duration, error } => {
                // We successfully connected - if this was a reconnect, record it immediately
                if needs_reconnect {
                    metrics.inc_reconnect_successes();
                }

                // Mark that any future connection is a reconnect
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

                // About to attempt reconnection
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
                // Failed to connect - track all connection failures
                metrics.inc_connection_failures();

                // Mark that any future connection is a reconnect attempt
                needs_reconnect = true;

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
    fn test_build_stream_url_single_production() {
        let symbols = vec!["BTCUSDT".to_string()];
        let url = build_stream_url(&symbols, BinanceEnvironment::Production, false, 100);
        assert_eq!(url, "wss://stream.binance.com:9443/ws/btcusdt@trade");
    }

    #[test]
    fn test_build_stream_url_multiple_production() {
        let symbols = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let url = build_stream_url(&symbols, BinanceEnvironment::Production, false, 100);
        assert_eq!(
            url,
            "wss://stream.binance.com:9443/stream?streams=btcusdt@trade/ethusdt@trade"
        );
    }

    #[test]
    fn test_build_stream_url_single_testnet() {
        let symbols = vec!["BTCUSDT".to_string()];
        let url = build_stream_url(&symbols, BinanceEnvironment::Testnet, false, 100);
        assert_eq!(
            url,
            "wss://stream.testnet.binance.vision:9443/ws/btcusdt@trade"
        );
    }

    #[test]
    fn test_build_stream_url_multiple_testnet() {
        let symbols = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let url = build_stream_url(&symbols, BinanceEnvironment::Testnet, false, 100);
        assert_eq!(
            url,
            "wss://stream.testnet.binance.vision:9443/stream?streams=btcusdt@trade/ethusdt@trade"
        );
    }

    #[test]
    fn test_build_stream_url_with_depth() {
        let symbols = vec!["BTCUSDT".to_string()];
        let url = build_stream_url(&symbols, BinanceEnvironment::Production, true, 100);
        assert_eq!(
            url,
            "wss://stream.binance.com:9443/stream?streams=btcusdt@trade/btcusdt@depth@100ms"
        );
    }

    #[test]
    fn test_build_stream_url_multiple_with_depth() {
        let symbols = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let url = build_stream_url(&symbols, BinanceEnvironment::Production, true, 1000);
        assert_eq!(
            url,
            "wss://stream.binance.com:9443/stream?streams=btcusdt@trade/btcusdt@depth@1000ms/ethusdt@trade/ethusdt@depth@1000ms"
        );
    }
}
