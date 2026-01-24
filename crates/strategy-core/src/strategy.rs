//! Strategy trait definition.

use std::time::Duration;

use async_trait::async_trait;
use execution_core::ExecutionReport;
use model::MarketEvent;

use crate::context::StrategyContext;
use crate::error::StrategyError;
use crate::signal::Signal;

/// Core trait for implementing trading strategies.
///
/// Strategies receive market events, execution reports, and timer callbacks,
/// and can respond by generating trading signals.
///
/// # Lifecycle
///
/// 1. `on_start` - Called once when the strategy runner starts
/// 2. `on_market_event` - Called for each market event (trades, order book updates, etc.)
/// 3. `on_order_update` - Called when an order status changes
/// 4. `on_timer` - Called at the configured timer interval (if set)
/// 5. `on_stop` - Called once when the strategy runner stops
///
/// # Example
///
/// ```rust,ignore
/// use async_trait::async_trait;
/// use strategy_core::{Strategy, StrategyContext, StrategyError, Signal, OrderIntent};
/// use model::MarketEvent;
/// use execution_core::ExecutionReport;
/// use rust_decimal_macros::dec;
///
/// struct MyStrategy {
///     id: String,
///     threshold: Decimal,
/// }
///
/// #[async_trait]
/// impl Strategy for MyStrategy {
///     fn id(&self) -> &str {
///         &self.id
///     }
///
///     async fn on_market_event(
///         &mut self,
///         event: &MarketEvent,
///         ctx: &StrategyContext,
///     ) -> Result<Option<Signal>, StrategyError> {
///         if let MarketEvent::Trade(trade) = event {
///             if trade.price < self.threshold {
///                 return Ok(Some(Signal::order(
///                     self.id(),
///                     OrderIntent::market_buy("BTCUSDT", dec!(0.001)),
///                 )));
///             }
///         }
///         Ok(None)
///     }
/// }
/// ```
#[async_trait]
pub trait Strategy: Send + Sync {
    /// Returns the unique identifier for this strategy.
    fn id(&self) -> &str;

    /// Returns the symbols this strategy is interested in.
    ///
    /// If `None`, the strategy receives events for all symbols.
    /// If `Some`, only events for the listed symbols are delivered.
    fn symbols(&self) -> Option<&[String]> {
        None
    }

    /// Returns the timer interval for periodic callbacks.
    ///
    /// If `Some(duration)`, `on_timer` will be called at this interval.
    /// If `None`, no timer callbacks occur.
    fn timer_interval(&self) -> Option<Duration> {
        None
    }

    /// Called once when the strategy runner starts.
    ///
    /// Use this to initialize state, load historical data, etc.
    async fn on_start(&mut self, _ctx: &StrategyContext) -> Result<(), StrategyError> {
        Ok(())
    }

    /// Called for each market event.
    ///
    /// This is the primary method for reacting to market data.
    /// Return `Ok(Some(signal))` to generate a trading signal.
    async fn on_market_event(
        &mut self,
        _event: &MarketEvent,
        _ctx: &StrategyContext,
    ) -> Result<Option<Signal>, StrategyError> {
        Ok(None)
    }

    /// Called when an order status changes.
    ///
    /// Use this to track fills, update position state, etc.
    async fn on_order_update(
        &mut self,
        _report: &ExecutionReport,
        _ctx: &StrategyContext,
    ) -> Result<Option<Signal>, StrategyError> {
        Ok(None)
    }

    /// Called at the configured timer interval.
    ///
    /// Use this for periodic tasks like:
    /// - Rebalancing
    /// - Risk checks
    /// - Position reconciliation
    async fn on_timer(&mut self, _ctx: &StrategyContext) -> Result<Option<Signal>, StrategyError> {
        Ok(None)
    }

    /// Called once when the strategy runner stops.
    ///
    /// Use this to clean up, cancel open orders, etc.
    async fn on_stop(&mut self, _ctx: &StrategyContext) -> Result<(), StrategyError> {
        Ok(())
    }
}

/// A boxed strategy trait object.
pub type BoxedStrategy = Box<dyn Strategy>;
