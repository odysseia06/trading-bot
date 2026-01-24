//! Binance REST API error types.

use auth::AuthError;
use rest_client::RestError;
use thiserror::Error;

/// Errors that can occur when interacting with the Binance REST API.
#[derive(Debug, Error)]
pub enum BinanceRestError {
    /// REST client error (network, timeout, etc.).
    #[error("REST client error: {0}")]
    Rest(#[from] RestError),

    /// Authentication error.
    #[error("Authentication error: {0}")]
    Auth(#[from] AuthError),

    /// Binance API error (returned by the exchange).
    #[error("Binance API error {code}: {message}")]
    ApiError {
        /// Binance error code.
        code: i32,
        /// Error message.
        message: String,
    },

    /// Invalid order parameters.
    #[error("Invalid order: {0}")]
    InvalidOrder(String),

    /// Order not found.
    #[error("Order not found")]
    OrderNotFound,

    /// Insufficient balance for the order.
    #[error("Insufficient balance")]
    InsufficientBalance,

    /// Listen key has expired or is invalid.
    #[error("Listen key expired or invalid")]
    ListenKeyExpired,

    /// Failed to parse response.
    #[error("Parse error: {0}")]
    Parse(String),
}

impl BinanceRestError {
    /// Parse a Binance API error response.
    ///
    /// Binance returns errors in the format: `{"code": -1000, "msg": "..."}`
    pub fn from_api_response(body: &str) -> Self {
        #[derive(serde::Deserialize)]
        struct ApiError {
            code: i32,
            msg: String,
        }

        match serde_json::from_str::<ApiError>(body) {
            Ok(err) => Self::classify_api_error(err.code, err.msg),
            Err(_) => Self::Parse(format!("Failed to parse error response: {}", body)),
        }
    }

    /// Classify a Binance API error code into a more specific error.
    fn classify_api_error(code: i32, message: String) -> Self {
        match code {
            // Listen key errors
            -1125 => Self::ListenKeyExpired,
            // Order errors
            -2010 => Self::InsufficientBalance,
            -2011 => Self::OrderNotFound,
            -2013 => Self::OrderNotFound,
            // Generic API error
            _ => Self::ApiError { code, message },
        }
    }

    /// Check if this error indicates the operation should be retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Rest(rest_err) => rest_err.is_retryable(),
            Self::ApiError { code, .. } => {
                // Binance server errors are often retryable
                matches!(code, -1000 | -1001 | -1003 | -1015 | -1016)
            }
            _ => false,
        }
    }
}
