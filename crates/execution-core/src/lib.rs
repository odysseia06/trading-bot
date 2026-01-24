//! Core execution types and utilities.
//!
//! This crate provides the fundamental types for order execution:
//!
//! - **Order types**: `Order`, `OrderSide`, `OrderType`, `OrderStatus`, `TimeInForce`
//! - **Execution reports**: `ExecutionReport` received from exchange WebSocket
//! - **Correlation registry**: `PendingOrderRegistry` for handling REST/WS race conditions
//!
//! # Order Lifecycle
//!
//! 1. Strategy generates a signal
//! 2. System generates a unique `client_order_id` and registers it in `PendingOrderRegistry`
//! 3. Order is submitted via REST API
//! 4. REST response arrives (may be beaten by WS)
//! 5. WebSocket `executionReport` arrives with status updates
//! 6. Order reaches terminal state (Filled, Canceled, Rejected, Expired)
//!
//! # Race Condition Handling
//!
//! The `PendingOrderRegistry` handles the common race condition where a WebSocket
//! execution report arrives before the REST API response. See the `correlation`
//! module documentation for details.

mod correlation;
mod execution;
mod order;
mod position;

pub use correlation::{
    create_pending_order_registry, generate_client_order_id, PendingOrderRegistry,
    PendingOrderState, SharedPendingOrderRegistry,
};
pub use execution::ExecutionReport;
pub use order::{Order, OrderSide, OrderStatus, OrderType, TimeInForce};
pub use position::{create_position_tracker, Position, PositionTracker, SharedPositionTracker};
