//! Order book error types.

use thiserror::Error;

/// Errors that can occur during order book operations.
#[derive(Debug, Error)]
pub enum OrderBookError {
    /// Sequence gap detected in delta update.
    #[error("sequence gap: expected {expected}, got {actual}")]
    SequenceGap { expected: u64, actual: u64 },

    /// Invalid price level (negative or zero price).
    #[error("invalid price: {0}")]
    InvalidPrice(String),

    /// Invalid quantity (negative quantity).
    #[error("invalid quantity: {0}")]
    InvalidQuantity(String),

    /// Order book not initialized with snapshot.
    #[error("order book not initialized - apply snapshot first")]
    NotInitialized,
}
