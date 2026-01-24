//! Generic REST client infrastructure.
//!
//! This crate provides a thin wrapper around `reqwest` with:
//!
//! - Consistent error handling via `RestError`
//! - Support for common HTTP methods (GET, POST, PUT, DELETE)
//! - JSON response deserialization
//! - Header injection for authentication
//! - Rate limit detection
//!
//! # Example
//!
//! ```rust,ignore
//! use rest_client::RestClient;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct TimeResponse {
//!     server_time: i64,
//! }
//!
//! let client = RestClient::with_default_timeout("https://api.binance.com")?;
//! let time: TimeResponse = client.get("/api/v3/time", None, None).await?;
//! ```

mod client;
mod error;

pub use client::RestClient;
pub use error::RestError;
