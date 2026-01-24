//! Core strategy types and traits for the trading bot.
//!
//! This crate provides the fundamental building blocks for implementing trading strategies:
//!
//! - **Strategy trait**: The core `Strategy` trait that all strategies must implement
//! - **Signal types**: `Signal`, `OrderIntent`, `CancelIntent`, `ModifyIntent` for expressing trading intent
//! - **Context**: `StrategyContext` providing read-only access to market state
//!
//! # Example Strategy
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//! use strategy_core::{Strategy, StrategyContext, StrategyError, Signal, OrderIntent};
//! use model::MarketEvent;
//! use rust_decimal_macros::dec;
//!
//! struct SimpleStrategy {
//!     id: String,
//! }
//!
//! #[async_trait]
//! impl Strategy for SimpleStrategy {
//!     fn id(&self) -> &str {
//!         &self.id
//!     }
//!
//!     async fn on_market_event(
//!         &mut self,
//!         event: &MarketEvent,
//!         ctx: &StrategyContext,
//!     ) -> Result<Option<Signal>, StrategyError> {
//!         // React to market events and optionally generate signals
//!         Ok(None)
//!     }
//! }
//! ```

mod context;
mod error;
mod signal;
mod strategy;

pub use context::{
    create_market_state, MarketState, OrderBookSnapshot, SharedMarketState, StrategyContext,
};
pub use error::StrategyError;
pub use signal::{CancelIntent, ModifyIntent, OrderIntent, Signal, SignalKind};
pub use strategy::{BoxedStrategy, Strategy};

// Re-export commonly used types from dependencies for convenience
pub use execution_core::{ExecutionReport, OrderSide, OrderStatus, OrderType, TimeInForce};
pub use model::MarketEvent;
pub use orderbook::PriceLevel;
