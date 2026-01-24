use connector_binance::run_connector;
use connector_core::{create_event_channel, ConnectorConfig};
use metrics::create_metrics;
use model::MarketEvent;
use std::time::Duration;
use tokio::sync::watch;
use tracing::info;

/// Interval for periodic health status logging.
const HEALTH_LOG_INTERVAL: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() {
    common::init_logging();

    let symbols = std::env::args().skip(1).collect::<Vec<_>>();

    let symbols = if symbols.is_empty() {
        vec!["BTCUSDT".to_string()]
    } else {
        symbols
    };

    info!(symbols = ?symbols, "Starting market connector");

    let config = ConnectorConfig {
        symbols,
        channel_capacity: 1024,
    };

    let (sender, mut receiver) = create_event_channel(config.channel_capacity);

    // Create metrics
    let metrics = create_metrics();

    // Create shutdown signal channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn connector task
    let connector_metrics = metrics.clone();
    let connector_handle = tokio::spawn(async move {
        if let Err(e) = run_connector(config, sender, shutdown_rx, connector_metrics).await {
            tracing::error!(error = %e, "Connector error");
        }
    });

    // Spawn ctrl_c handler
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("Received Ctrl+C, initiating shutdown");
            let _ = shutdown_tx_clone.send(true);
        }
    });

    // Spawn periodic health reporter
    let health_metrics = metrics.clone();
    let health_shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEALTH_LOG_INTERVAL);
        let mut shutdown_rx = health_shutdown_rx;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let snapshot = health_metrics.snapshot();
                    let status = snapshot.health_status();
                    info!(
                        status = %status,
                        trades = snapshot.trades_received,
                        trades_per_sec = format!("{:.1}", snapshot.trades_per_second),
                        errors = snapshot.websocket_errors + snapshot.parse_errors,
                        reconnects = snapshot.reconnect_attempts,
                        "Health check"
                    );
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    });

    // Print trades as they arrive
    while let Some(event) = receiver.recv().await {
        match event {
            MarketEvent::Trade(trade) => {
                println!(
                    "{} | {} | price: {} | qty: {} | side: {:?}",
                    trade.timestamp_ms, trade.symbol, trade.price, trade.qty, trade.maker_side
                );
            }
        }
    }

    info!("Event channel closed, waiting for connector to finish");

    // Wait for connector to finish
    let _ = connector_handle.await;

    // Print final metrics
    let snapshot = metrics.snapshot();
    println!("\n{}", snapshot);

    info!("Shutdown complete");
}
