//! REST client error types.

use thiserror::Error;

/// Errors that can occur during REST API calls.
#[derive(Debug, Error)]
pub enum RestError {
    /// HTTP error with status code and message.
    #[error("HTTP error: {status} - {message}")]
    HttpError {
        /// HTTP status code.
        status: u16,
        /// Error message or response body.
        message: String,
    },

    /// Request timed out.
    #[error("Request timeout")]
    Timeout,

    /// Connection error (network issue).
    #[error("Connection error: {0}")]
    Connection(String),

    /// Failed to parse response body as JSON.
    #[error("JSON parse error: {0}")]
    Parse(String),

    /// Rate limited by the server.
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited {
        /// Suggested wait time before retrying.
        retry_after_ms: u64,
    },

    /// Failed to build the HTTP request.
    #[error("Request build error: {0}")]
    RequestBuild(String),
}

impl RestError {
    /// Check if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            RestError::Timeout | RestError::Connection(_) | RestError::RateLimited { .. }
        )
    }

    /// Check if this is a rate limit error.
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, RestError::RateLimited { .. })
    }
}

impl From<reqwest::Error> for RestError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            RestError::Timeout
        } else if err.is_connect() {
            RestError::Connection(err.to_string())
        } else if err.is_decode() {
            RestError::Parse(err.to_string())
        } else if let Some(status) = err.status() {
            RestError::HttpError {
                status: status.as_u16(),
                message: err.to_string(),
            }
        } else {
            RestError::Connection(err.to_string())
        }
    }
}
