//! Binance REST API client.
//!
//! This crate provides a typed client for the Binance REST API with:
//!
//! - **Time synchronization**: Adjusts for clock skew between local and server time
//! - **Listen key management**: Create, refresh, and close user data stream keys
//! - **Order management**: Place, query, and cancel orders with proper signing
//! - **Error handling**: Typed errors with specific variants for common cases
//!
//! # Example
//!
//! ```rust,ignore
//! use auth::ApiCredentials;
//! use binance_rest::BinanceRestClient;
//!
//! // Load credentials from environment
//! let credentials = ApiCredentials::from_env()?;
//! let client = BinanceRestClient::new(credentials)?;
//!
//! // Sync time with Binance server
//! client.sync_time().await?;
//!
//! // Create a listen key for user data stream
//! let listen_key = client.create_listen_key().await?;
//!
//! // Place an order
//! let response = client.place_order(
//!     "BTCUSDT",
//!     OrderSide::Buy,
//!     OrderType::Limit,
//!     dec!(0.001),
//!     Some(dec!(50000.0)),
//!     Some(TimeInForce::GTC),
//!     "my_order_123",
//! ).await?;
//! ```

mod client;
mod error;
mod responses;

pub use client::BinanceRestClient;
pub use error::BinanceRestError;
pub use responses::{
    ListenKeyResponse, NewOrderResponse, OrderFill, OrderQueryResponse, ServerTimeResponse,
};
