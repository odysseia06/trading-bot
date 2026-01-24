//! Strategy runner error types.

use thiserror::Error;

/// Errors that can occur during strategy runner execution.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// Strategy error.
    #[error("strategy error: {0}")]
    Strategy(#[from] strategy_core::StrategyError),

    /// Signal processing error.
    #[error("signal processing error: {0}")]
    SignalProcessing(#[from] SignalError),

    /// REST API error.
    #[error("REST API error: {0}")]
    RestApi(#[from] binance_rest::BinanceRestError),

    /// Channel closed unexpectedly.
    #[error("channel closed")]
    ChannelClosed,

    /// Shutdown requested.
    #[error("shutdown requested")]
    Shutdown,
}

/// Errors that can occur during signal processing.
#[derive(Debug, Error)]
pub enum SignalError {
    /// Rate limit exceeded.
    #[error("rate limit exceeded: {0} orders/sec")]
    RateLimitExceeded(u32),

    /// Order notional too small.
    #[error("order notional {0} below minimum {1}")]
    NotionalTooSmall(String, String),

    /// Order notional too large.
    #[error("order notional {0} above maximum {1}")]
    NotionalTooLarge(String, String),

    /// Invalid order parameters.
    #[error("invalid order: {0}")]
    InvalidOrder(String),

    /// Missing price for limit order.
    #[error("limit order requires price")]
    MissingPrice,

    /// Order ID generation failed.
    #[error("failed to generate order ID")]
    OrderIdGeneration,
}
