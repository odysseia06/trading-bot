//! Local order book implementation for market depth tracking.
//!
//! This crate provides a high-performance order book implementation using
//! sorted `BTreeMap` structures for efficient price level access.
//!
//! # Example
//!
//! ```rust
//! use orderbook::OrderBook;
//! use rust_decimal_macros::dec;
//!
//! let mut book = OrderBook::new("BTCUSDT");
//!
//! // Apply initial snapshot
//! let bids = vec![(dec!(100.0), dec!(1.0)), (dec!(99.0), dec!(2.0))];
//! let asks = vec![(dec!(101.0), dec!(1.5)), (dec!(102.0), dec!(2.5))];
//! book.apply_snapshot(&bids, &asks, 1000);
//!
//! // Access market data
//! println!("Best bid: {:?}", book.best_bid());
//! println!("Best ask: {:?}", book.best_ask());
//! println!("Mid price: {:?}", book.mid_price());
//! println!("Spread: {:?}", book.spread());
//! ```

mod book;
mod error;
mod level;

pub use book::OrderBook;
pub use error::OrderBookError;
pub use level::PriceLevel;
