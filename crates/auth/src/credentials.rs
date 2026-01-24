//! Secure API credential management.
//!
//! Uses the `secrecy` crate to prevent accidental logging of secret keys
//! and ensures memory is zeroed on drop.

use crate::error::AuthError;
use secrecy::{ExposeSecret, SecretString};

/// API credentials for authenticated requests.
///
/// The secret key is wrapped in `SecretString` which:
/// - Prevents accidental Debug/Display printing
/// - Zeros memory on drop via zeroize
#[derive(Clone)]
pub struct ApiCredentials {
    api_key: String,
    secret_key: SecretString,
}

impl ApiCredentials {
    /// Load credentials from environment variables.
    ///
    /// Looks for:
    /// - `BINANCE_API_KEY` - The API key (public)
    /// - `BINANCE_SECRET_KEY` - The secret key (private)
    ///
    /// # Errors
    /// Returns `AuthError::MissingEnvVar` if either variable is not set.
    pub fn from_env() -> Result<Self, AuthError> {
        // Load .env file if present (ignores errors if file doesn't exist)
        dotenvy::dotenv().ok();

        let api_key = std::env::var("BINANCE_API_KEY")
            .map_err(|_| AuthError::MissingEnvVar("BINANCE_API_KEY".into()))?;

        let secret_key = std::env::var("BINANCE_SECRET_KEY")
            .map_err(|_| AuthError::MissingEnvVar("BINANCE_SECRET_KEY".into()))?;

        Ok(Self::new(api_key, secret_key))
    }

    /// Create credentials from explicit values.
    ///
    /// Useful for testing or when credentials come from other sources.
    pub fn new(api_key: String, secret_key: String) -> Self {
        Self {
            api_key,
            secret_key: SecretString::from(secret_key),
        }
    }

    /// Get the API key (public, safe to log).
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Expose the secret key for signing.
    ///
    /// **WARNING**: Only use this for cryptographic operations.
    /// Never log or display the return value.
    pub fn expose_secret(&self) -> &str {
        self.secret_key.expose_secret()
    }
}

impl std::fmt::Debug for ApiCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiCredentials")
            .field("api_key", &self.api_key)
            .field("secret_key", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_new() {
        let creds = ApiCredentials::new("my_api_key".into(), "my_secret".into());
        assert_eq!(creds.api_key(), "my_api_key");
        assert_eq!(creds.expose_secret(), "my_secret");
    }

    #[test]
    fn test_debug_redacts_secret() {
        let creds = ApiCredentials::new("my_api_key".into(), "super_secret_key".into());
        let debug_str = format!("{:?}", creds);

        assert!(debug_str.contains("my_api_key"));
        assert!(!debug_str.contains("super_secret_key"));
        assert!(debug_str.contains("[REDACTED]"));
    }
}
