//! Strategy execution runtime for the trading bot.
//!
//! This crate provides the runtime for executing trading strategies:
//!
//! - **StrategyRunner**: Main execution loop that dispatches events to strategies
//! - **SignalProcessor**: Validates signals and enforces rate limits
//! - **TimerManager**: Manages periodic callbacks for strategies
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌──────────────────┐     ┌───────────────┐
//! │  Strategy   │────>│ SignalProcessor  │────>│ BinanceRest   │
//! │  on_market_ │     │ - validate       │     │ - place_order │
//! │  event()    │     │ - rate limit     │     │ - cancel_order│
//! └─────────────┘     │ - gen order ID   │     └───────────────┘
//!        ^            └──────────────────┘            │
//!        │                                            v
//!        │                                   ┌───────────────┐
//!        └───────────────────────────────────│ ExecutionRpt  │
//!               on_order_update()            │ (WebSocket)   │
//!                                            └───────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use strategy_runner::{StrategyRunner, StrategyRunnerConfig};
//!
//! // Create runner with default config (dry-run mode)
//! let config = StrategyRunnerConfig::default();
//! let mut runner = StrategyRunner::new(config);
//!
//! // Register strategies
//! runner.register_strategy(Box::new(my_strategy));
//!
//! // Run the event loop
//! runner.run(market_rx, execution_rx, shutdown_rx).await?;
//! ```

mod error;
pub mod examples;
mod runner;
mod signal_processor;
mod timer;

pub use error::{RunnerError, SignalError};
pub use runner::{StrategyRunner, StrategyRunnerConfig};
pub use signal_processor::{ProcessedSignal, SignalProcessor, SignalProcessorConfig};
pub use timer::TimerManager;
