//! User data stream WebSocket client.
//!
//! Connects to Binance user data stream to receive:
//! - Order execution reports
//! - Account balance updates
//!
//! Handles listen key lifecycle (creation, refresh, expiry).

use crate::user_data_parser::{parse_user_data_message, AccountUpdate, UserDataMessage};
use binance_rest::BinanceRestClient;
use common::ExponentialBackoff;
use connector_core::ConnectorError;
use execution_core::{ExecutionReport, SharedPendingOrderRegistry};
use futures_util::{SinkExt, StreamExt};
use metrics::SharedMetrics;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

const BINANCE_WS_BASE: &str = "wss://stream.binance.com:9443";

/// Listen key refresh interval (30 minutes).
/// Keys expire after 60 minutes, so we refresh at half that time.
const LISTEN_KEY_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Connection timeout for WebSocket.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Callback for sending execution reports to the main application.
pub type ExecutionReportCallback = Box<dyn Fn(ExecutionReport) + Send + Sync>;

/// Callback for sending account updates to the main application.
pub type AccountUpdateCallback = Box<dyn Fn(AccountUpdate) + Send + Sync>;

/// Result of a user data stream session.
enum SessionResult {
    /// Shutdown was requested
    Shutdown,
    /// Connection error occurred
    Error(ConnectorError),
    /// Listen key expired or became invalid
    ListenKeyExpired,
}

/// Run the user data stream with automatic listen key refresh and reconnection.
///
/// This function:
/// 1. Creates a listen key via REST API
/// 2. Connects to the user data WebSocket stream
/// 3. Refreshes the listen key every 30 minutes
/// 4. Handles reconnection with exponential backoff
///
/// # Arguments
/// * `rest_client` - Binance REST client for listen key management
/// * `on_execution_report` - Callback when an execution report is received
/// * `on_account_update` - Callback when an account update is received
/// * `shutdown_rx` - Shutdown signal receiver
/// * `metrics` - Metrics collector
/// * `pending_orders` - Registry for correlating orders
pub async fn run_user_data_stream(
    rest_client: Arc<BinanceRestClient>,
    on_execution_report: ExecutionReportCallback,
    on_account_update: AccountUpdateCallback,
    mut shutdown_rx: watch::Receiver<bool>,
    metrics: SharedMetrics,
    pending_orders: SharedPendingOrderRegistry,
) -> Result<(), ConnectorError> {
    let mut backoff = ExponentialBackoff::default();

    loop {
        if *shutdown_rx.borrow() {
            info!("Shutdown requested, exiting user data stream");
            return Ok(());
        }

        // Create listen key
        let listen_key = match rest_client.create_listen_key().await {
            Ok(lk) => {
                backoff.reset(); // Successful REST call, reset backoff
                lk
            }
            Err(e) => {
                warn!(error = %e, "Failed to create listen key");
                metrics.inc_websocket_errors();

                let delay = backoff.next_delay();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return Ok(());
                        }
                    }
                }
                continue;
            }
        };

        info!("Created listen key, connecting to user data stream");

        // Run session with this listen key
        match run_user_data_session(
            &listen_key,
            &rest_client,
            &on_execution_report,
            &on_account_update,
            &mut shutdown_rx,
            &metrics,
            &pending_orders,
        )
        .await
        {
            SessionResult::Shutdown => {
                // Clean up listen key
                if let Err(e) = rest_client.close_listen_key(&listen_key).await {
                    warn!(error = %e, "Failed to close listen key during shutdown");
                }
                return Ok(());
            }
            SessionResult::Error(e) => {
                warn!(error = %e, "User data session error, reconnecting");
                metrics.inc_reconnect_attempts();

                let delay = backoff.next_delay();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return Ok(());
                        }
                    }
                }
            }
            SessionResult::ListenKeyExpired => {
                info!("Listen key expired, creating new one");
                metrics.inc_reconnect_attempts();
                // No backoff needed for key expiry, just get a new key
            }
        }
    }
}

/// Run a single user data stream session.
async fn run_user_data_session(
    listen_key: &str,
    rest_client: &Arc<BinanceRestClient>,
    on_execution_report: &ExecutionReportCallback,
    on_account_update: &AccountUpdateCallback,
    shutdown_rx: &mut watch::Receiver<bool>,
    metrics: &SharedMetrics,
    pending_orders: &SharedPendingOrderRegistry,
) -> SessionResult {
    let url = format!("{}/ws/{}", BINANCE_WS_BASE, listen_key);

    // Connect with timeout
    let ws_stream = match tokio::time::timeout(CONNECTION_TIMEOUT, connect_async(&url)).await {
        Ok(Ok((stream, _))) => stream,
        Ok(Err(e)) => {
            return SessionResult::Error(ConnectorError::WebSocket(e.to_string()));
        }
        Err(_) => {
            return SessionResult::Error(ConnectorError::WebSocket("connection timeout".into()));
        }
    };

    info!("Connected to user data stream");
    metrics.inc_reconnect_successes();

    let (mut write, mut read) = ws_stream.split();
    let mut keepalive_interval = tokio::time::interval(LISTEN_KEY_REFRESH_INTERVAL);
    // Skip the first immediate tick
    keepalive_interval.tick().await;

    loop {
        tokio::select! {
            biased;

            // Check for shutdown
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Shutdown signal received, closing user data stream");
                    let _ = write.close().await;
                    return SessionResult::Shutdown;
                }
            }

            // Keep-alive timer
            _ = keepalive_interval.tick() => {
                match rest_client.keepalive_listen_key(listen_key).await {
                    Ok(()) => {
                        debug!("Listen key refreshed");
                    }
                    Err(e) => {
                        // Check if key expired
                        if matches!(e, binance_rest::BinanceRestError::ListenKeyExpired) {
                            warn!("Listen key expired during refresh");
                            return SessionResult::ListenKeyExpired;
                        }
                        warn!(error = %e, "Failed to refresh listen key, will retry");
                        // Don't disconnect, just try again next interval
                    }
                }
            }

            // WebSocket messages
            msg_opt = read.next() => {
                match msg_opt {
                    Some(Ok(Message::Text(text))) => {
                        metrics.inc_messages_received();

                        match parse_user_data_message(&text) {
                            Ok(UserDataMessage::ExecutionReport(report)) => {
                                debug!(
                                    client_order_id = %report.client_order_id,
                                    status = ?report.order_status,
                                    "Received execution report"
                                );

                                // Correlate with pending orders
                                let was_acked = pending_orders.on_ws_execution_report(report.clone());
                                if !was_acked {
                                    debug!(
                                        client_order_id = %report.client_order_id,
                                        "WS arrived before REST response"
                                    );
                                }

                                // Notify callback
                                on_execution_report(report);
                            }
                            Ok(UserDataMessage::AccountUpdate(update)) => {
                                debug!(
                                    num_balances = update.balances.len(),
                                    "Received account update"
                                );
                                on_account_update(update);
                            }
                            Ok(UserDataMessage::Unknown) => {
                                // Ignore unknown messages
                            }
                            Err(e) => {
                                metrics.inc_parse_errors();
                                warn!(error = %e, text = %text, "Failed to parse user data message");
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        debug!("Received Ping, sending Pong");
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            warn!(error = %e, "Failed to send Pong");
                            metrics.inc_websocket_errors();
                            return SessionResult::Error(ConnectorError::WebSocket(e.to_string()));
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("WebSocket closed by server");
                        return SessionResult::ListenKeyExpired;
                    }
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket error");
                        metrics.inc_websocket_errors();
                        return SessionResult::Error(ConnectorError::WebSocket(e.to_string()));
                    }
                    None => {
                        info!("WebSocket stream ended");
                        return SessionResult::ListenKeyExpired;
                    }
                    _ => {}
                }
            }
        }
    }
}
