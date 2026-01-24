//! Authentication and signing for exchange APIs.
//!
//! This crate provides secure credential management and request signing
//! for authenticated API calls to cryptocurrency exchanges.
//!
//! # Features
//!
//! - **Secure Credentials**: API secrets are wrapped in `SecretString` to prevent
//!   accidental logging and ensure memory is zeroed on drop.
//! - **HMAC-SHA256 Signing**: Implements the signing algorithm required by Binance.
//! - **Environment Loading**: Credentials can be loaded from environment variables
//!   or a `.env` file.
//!
//! # Example
//!
//! ```rust,ignore
//! use auth::{ApiCredentials, RequestSigner};
//!
//! // Load credentials from environment
//! let credentials = ApiCredentials::from_env()?;
//!
//! // Create a signer
//! let signer = RequestSigner::new(&credentials);
//!
//! // Sign a query string
//! let params = [("symbol", "BTCUSDT"), ("side", "BUY")];
//! let signed_query = signer.sign_params(&params, timestamp_ms);
//! ```

mod credentials;
mod error;
mod signer;

pub use credentials::ApiCredentials;
pub use error::AuthError;
pub use signer::RequestSigner;
