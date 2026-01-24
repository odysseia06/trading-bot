use thiserror::Error;

/// Errors that can occur during authentication operations.
#[derive(Debug, Error)]
pub enum AuthError {
    /// A required environment variable is missing.
    #[error("Missing environment variable: {0}")]
    MissingEnvVar(String),

    /// The API key format is invalid.
    #[error("Invalid API key format")]
    InvalidKeyFormat,
}
