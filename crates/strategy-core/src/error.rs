//! Strategy error types.

use thiserror::Error;

/// Errors that can occur during strategy execution.
#[derive(Debug, Error)]
pub enum StrategyError {
    /// Strategy initialization failed.
    #[error("initialization failed: {0}")]
    InitializationFailed(String),

    /// Invalid configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// Market data not available.
    #[error("market data not available for {0}")]
    NoMarketData(String),

    /// Internal strategy error.
    #[error("internal error: {0}")]
    Internal(String),
}
