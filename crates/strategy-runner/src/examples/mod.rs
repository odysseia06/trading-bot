//! Example strategy implementations.
//!
//! These strategies demonstrate how to implement the `Strategy` trait
//! and can be used as templates for custom strategies.

mod price_threshold;

pub use price_threshold::{PriceThresholdConfig, PriceThresholdStrategy};
