//! Strategy runner error types.

use rust_decimal::Decimal;
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

    /// Risk check rejected the order.
    #[error("risk rejected: {0}")]
    RiskRejected(#[from] RiskRejection),
}

/// Reasons why a risk check rejected an order.
#[derive(Debug, Clone, Error)]
pub enum RiskRejection {
    /// Kill switch is active - all trading halted.
    #[error("kill switch is active")]
    KillSwitchActive,

    /// Circuit breaker is active after a loss event.
    #[error("circuit breaker active, resumes at {resumes_at_ms}")]
    CircuitBreakerActive {
        /// Timestamp (ms) when trading can resume.
        resumes_at_ms: i64,
    },

    /// Daily loss limit has been exceeded.
    #[error("daily loss limit exceeded: current {current}, limit {limit}")]
    DailyLossLimitExceeded {
        /// Current daily loss (negative value).
        current: Decimal,
        /// Maximum allowed daily loss.
        limit: Decimal,
    },

    /// Drawdown limit has been exceeded.
    #[error("drawdown limit exceeded: current {current_pct}%, limit {limit_pct}%")]
    DrawdownLimitExceeded {
        /// Current drawdown percentage.
        current_pct: Decimal,
        /// Maximum allowed drawdown percentage.
        limit_pct: Decimal,
    },

    /// Position limit for a symbol would be exceeded.
    #[error("position limit exceeded for {symbol}: current {current}, limit {limit}")]
    PositionLimitExceeded {
        /// Trading pair symbol.
        symbol: String,
        /// Current position notional.
        current: Decimal,
        /// Maximum allowed position notional.
        limit: Decimal,
    },

    /// Total exposure limit would be exceeded.
    #[error("total exposure limit exceeded: current {current}, limit {limit}")]
    TotalExposureLimitExceeded {
        /// Current total exposure.
        current: Decimal,
        /// Maximum allowed total exposure.
        limit: Decimal,
    },

    /// Too many open orders for a symbol.
    #[error("too many open orders for {symbol}: current {current}, limit {limit}")]
    TooManyOpenOrdersPerSymbol {
        /// Trading pair symbol.
        symbol: String,
        /// Current number of open orders.
        current: u32,
        /// Maximum allowed open orders.
        limit: u32,
    },

    /// Too many open orders total.
    #[error("too many open orders total: current {current}, limit {limit}")]
    TooManyOpenOrdersTotal {
        /// Current total number of open orders.
        current: u32,
        /// Maximum allowed total open orders.
        limit: u32,
    },

    /// Single order is too large.
    #[error("order too large: notional {notional}, limit {limit}")]
    OrderTooLarge {
        /// Order notional value.
        notional: Decimal,
        /// Maximum allowed order notional.
        limit: Decimal,
    },
}
