//! Market connector runner.
//!
//! This is the main entry point for the trading bot. It:
//! - Connects to Binance market data stream (public)
//! - Optionally connects to user data stream (requires API keys)
//! - Runs the strategy engine with registered strategies
//! - Handles graceful shutdown on Ctrl+C
//! - Reports health metrics periodically
//!
//! # Usage
//!
//! ```bash
//! # Run with production (default)
//! cargo run --release -- BTCUSDT ETHUSDT
//!
//! # Run with testnet
//! BINANCE_ENVIRONMENT=testnet cargo run --release -- BTCUSDT
//!
//! # Or use --testnet flag
//! cargo run --release -- --testnet BTCUSDT ETHUSDT
//! ```

use auth::ApiCredentials;
use binance_rest::BinanceRestClient;
use common::BinanceEnvironment;
use connector_binance::{run_connector, run_user_data_stream, AccountUpdate};
use connector_core::{create_event_channel, ConnectorConfig};
use execution_core::{create_pending_order_registry, ExecutionReport};
use metrics::create_metrics;
use rust_decimal_macros::dec;
use std::sync::Arc;
use std::time::Duration;
use strategy_runner::{
    examples::{PriceThresholdConfig, PriceThresholdStrategy},
    SignalProcessorConfig, StrategyRunner, StrategyRunnerConfig,
};
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

/// Interval for periodic health status logging.
const HEALTH_LOG_INTERVAL: Duration = Duration::from_secs(60);

fn print_usage() {
    eprintln!("Usage: trading-bot [OPTIONS] [SYMBOLS...]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --testnet     Use Binance testnet (fake money)");
    eprintln!("  --live        Enable live trading (requires API keys)");
    eprintln!("  --help        Show this help message");
    eprintln!();
    eprintln!("Environment variables:");
    eprintln!("  BINANCE_API_KEY       API key for authenticated requests");
    eprintln!("  BINANCE_SECRET_KEY    Secret key for signing requests");
    eprintln!("  BINANCE_ENVIRONMENT   'production' (default) or 'testnet'");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  trading-bot                       # Stream BTCUSDT on production");
    eprintln!("  trading-bot ETHUSDT BNBUSDT       # Stream multiple symbols");
    eprintln!("  trading-bot --testnet BTCUSDT     # Use testnet");
    eprintln!("  trading-bot --testnet --live      # Live trading on testnet");
}

#[tokio::main]
async fn main() {
    // Load .env file if present (before anything else)
    match dotenvy::dotenv() {
        Ok(path) => eprintln!("Loaded environment from: {}", path.display()),
        Err(dotenvy::Error::Io(_)) => {} // No .env file, that's fine
        Err(e) => eprintln!("Warning: Failed to load .env file: {}", e),
    }

    common::init_logging();

    // Parse command line arguments
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut use_testnet = false;
    let mut live_trading = false;
    let mut symbols = Vec::new();

    for arg in &args {
        match arg.as_str() {
            "--testnet" | "-t" => use_testnet = true,
            "--live" | "-l" => live_trading = true,
            "--help" | "-h" => {
                print_usage();
                return;
            }
            s if s.starts_with('-') => {
                eprintln!("Unknown option: {}", s);
                print_usage();
                std::process::exit(1);
            }
            symbol => symbols.push(symbol.to_string()),
        }
    }

    // Determine environment: CLI flag takes precedence, then env var, then default
    let environment = if use_testnet {
        BinanceEnvironment::Testnet
    } else {
        BinanceEnvironment::from_env()
    };

    // Default symbol if none specified
    if symbols.is_empty() {
        symbols.push("BTCUSDT".to_string());
    }

    // Safety check: require --testnet for live trading unless explicitly on production
    if live_trading && environment.is_production() {
        warn!("Live trading on PRODUCTION with REAL MONEY!");
        warn!("Press Ctrl+C within 5 seconds to abort...");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    info!(
        environment = %environment,
        symbols = ?symbols,
        live_trading = live_trading,
        "Starting trading bot"
    );

    // Try to load API credentials from environment
    let credentials = match ApiCredentials::from_env() {
        Ok(creds) => {
            info!(api_key = %creds.api_key(), "Loaded API credentials");
            Some(creds)
        }
        Err(e) => {
            info!(
                reason = %e,
                "No API credentials found, running in read-only mode (market data only)"
            );
            if live_trading {
                error!(
                    "--live requires API credentials. Set BINANCE_API_KEY and BINANCE_SECRET_KEY"
                );
                std::process::exit(1);
            }
            None
        }
    };

    let config = ConnectorConfig {
        symbols: symbols.clone(),
        channel_capacity: 1024,
        environment,
    };

    let (sender, receiver) = create_event_channel(config.channel_capacity);

    // Create shared resources
    let metrics = create_metrics();
    let pending_orders = create_pending_order_registry();

    // Create shutdown signal channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Create channel for execution reports to strategy runner
    let (execution_tx, execution_rx) = mpsc::channel::<ExecutionReport>(256);

    // Spawn market data connector task
    let connector_metrics = metrics.clone();
    let market_shutdown_rx = shutdown_rx.clone();
    let connector_handle = tokio::spawn(async move {
        if let Err(e) = run_connector(config, sender, market_shutdown_rx, connector_metrics).await {
            error!(error = %e, "Market connector error");
        }
    });

    // Spawn user data stream task (if credentials available)
    let rest_client: Option<Arc<BinanceRestClient>> = if let Some(ref creds) = credentials {
        match BinanceRestClient::with_environment(creds.clone(), environment) {
            Ok(client) => {
                let client = Arc::new(client);
                info!(
                    environment = %client.environment(),
                    "Created REST client"
                );
                // Sync time with Binance server
                if let Err(e) = client.sync_time().await {
                    warn!(error = %e, "Failed to sync server time, timestamps may be rejected");
                }
                Some(client)
            }
            Err(e) => {
                error!(error = %e, "Failed to create REST client");
                None
            }
        }
    } else {
        None
    };

    let user_data_handle = if let Some(ref client) = rest_client {
        let user_metrics = metrics.clone();
        let user_shutdown_rx = shutdown_rx.clone();
        let user_pending_orders = pending_orders.clone();
        let client = Arc::clone(client);
        let execution_tx = execution_tx.clone();

        // Create callbacks for execution reports and account updates
        let on_execution_report: Box<dyn Fn(ExecutionReport) + Send + Sync> =
            Box::new(move |report| {
                info!(
                    symbol = %report.symbol,
                    client_order_id = %report.client_order_id,
                    status = ?report.order_status,
                    filled_qty = %report.cumulative_filled_qty,
                    "Execution report"
                );
                // Forward to strategy runner
                let _ = execution_tx.try_send(report);
            });

        let on_account_update: Box<dyn Fn(AccountUpdate) + Send + Sync> = Box::new(move |update| {
            for balance in &update.balances {
                if balance.free > rust_decimal::Decimal::ZERO
                    || balance.locked > rust_decimal::Decimal::ZERO
                {
                    info!(
                        asset = %balance.asset,
                        free = %balance.free,
                        locked = %balance.locked,
                        "Balance update"
                    );
                }
            }
        });

        Some(tokio::spawn(async move {
            if let Err(e) = run_user_data_stream(
                client,
                on_execution_report,
                on_account_update,
                user_shutdown_rx,
                user_metrics,
                user_pending_orders,
            )
            .await
            {
                error!(error = %e, "User data stream error");
            }
        }))
    } else {
        None
    };

    // Create and configure strategy runner
    let strategy_config = StrategyRunnerConfig {
        signal_processor: SignalProcessorConfig {
            max_orders_per_second: 5,
            min_order_notional: dec!(10),
            max_order_notional: dec!(100000),
            order_id_prefix: "bot".to_string(),
            live_trading,
        },
        stale_order_timeout_ms: 300_000, // 5 minutes
    };

    let mut strategy_runner = StrategyRunner::new(strategy_config);

    // Optionally add REST client for live trading
    if let Some(ref client) = rest_client {
        strategy_runner = strategy_runner
            .with_rest_client(Arc::clone(client))
            .with_pending_orders(pending_orders.clone());
    }

    // Register example strategy (price threshold for first symbol)
    let primary_symbol = symbols
        .first()
        .cloned()
        .unwrap_or_else(|| "BTCUSDT".to_string());
    let price_threshold_config = PriceThresholdConfig {
        symbol: primary_symbol.clone(),
        buy_threshold: dec!(90000),   // Buy below $90,000
        sell_threshold: dec!(100000), // Sell above $100,000
        quantity: dec!(0.001),
        cooldown_ms: 60_000, // 1 minute cooldown between signals
    };

    let strategy = PriceThresholdStrategy::new("price_threshold_1", price_threshold_config);
    strategy_runner.register_strategy(Box::new(strategy));

    let mode = if live_trading { "LIVE" } else { "DRY RUN" };
    info!(
        symbol = %primary_symbol,
        buy_threshold = 90000,
        sell_threshold = 100000,
        mode = mode,
        "Registered price threshold strategy"
    );

    // Spawn strategy runner task
    let strategy_shutdown_rx = shutdown_rx.clone();
    let strategy_handle = tokio::spawn(async move {
        if let Err(e) = strategy_runner
            .run(receiver, execution_rx, strategy_shutdown_rx)
            .await
        {
            error!(error = %e, "Strategy runner error");
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

    info!("Trading bot running. Press Ctrl+C to stop.");

    // Wait for strategy runner to finish (it owns the market event receiver)
    let _ = strategy_handle.await;

    info!("Strategy runner stopped, waiting for other tasks");

    // Wait for connector to finish
    let _ = connector_handle.await;

    // Wait for user data stream to finish (if running)
    if let Some(handle) = user_data_handle {
        let _ = handle.await;
    }

    // Print final metrics
    let snapshot = metrics.snapshot();
    println!("\n{}", snapshot);

    info!("Shutdown complete");
}
