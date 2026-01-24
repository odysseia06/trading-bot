//! Client order ID correlation for handling REST/WebSocket race conditions.
//!
//! When placing an order:
//! 1. Generate unique clientOrderId
//! 2. Pre-register in PendingOrderRegistry (state: AwaitingAck)
//! 3. Send REST request
//! 4. Either:
//!    a. REST response arrives first -> update state to RestAcked
//!    b. WS executionReport arrives first -> store it, return when REST arrives

use crate::execution::ExecutionReport;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Generate a unique client order ID with a prefix.
///
/// Format: `{prefix}_{uuid}` where uuid is a v4 UUID in simple format (no hyphens).
pub fn generate_client_order_id(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().as_simple())
}

/// State of a pending order in the correlation registry.
#[derive(Debug)]
pub enum PendingOrderState {
    /// Order sent, waiting for acknowledgment from REST or WS.
    AwaitingAck {
        /// Timestamp when the order was created.
        created_at_ms: i64,
    },
    /// REST response received, order acknowledged by exchange.
    RestAcked {
        /// Exchange-assigned order ID.
        exchange_order_id: u64,
    },
    /// WS executionReport arrived before REST response.
    WsArrivedFirst {
        /// The execution report that arrived early.
        execution_report: ExecutionReport,
    },
}

/// Thread-safe registry for correlating orders between REST and WebSocket.
///
/// This handles the race condition where a WebSocket executionReport may
/// arrive before the REST API response returns.
///
/// # Usage
///
/// ```rust,ignore
/// let registry = PendingOrderRegistry::new();
///
/// // Before sending REST request
/// let client_order_id = generate_client_order_id("BOT");
/// registry.register(&client_order_id, current_time_ms);
///
/// // Send REST request...
/// let response = client.place_order(...).await?;
///
/// // When REST response arrives
/// if let Some(early_report) = registry.on_rest_response(&client_order_id, response.order_id) {
///     // WS arrived first, process the early report
/// }
///
/// // When WS executionReport arrives (in another task)
/// let was_acked = registry.on_ws_execution_report(report);
/// if was_acked {
///     // Normal flow: REST already acknowledged
/// } else {
///     // WS arrived first, report is stored in registry
/// }
/// ```
pub struct PendingOrderRegistry {
    pending: DashMap<String, PendingOrderState>,
}

impl Default for PendingOrderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingOrderRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            pending: DashMap::new(),
        }
    }

    /// Pre-register an order before sending the REST request.
    ///
    /// This must be called before the REST request is sent to handle
    /// the case where WS arrives before REST.
    pub fn register(&self, client_order_id: &str, created_at_ms: i64) {
        self.pending.insert(
            client_order_id.to_string(),
            PendingOrderState::AwaitingAck { created_at_ms },
        );
    }

    /// Called when REST response arrives.
    ///
    /// Returns any execution report that arrived via WebSocket first.
    pub fn on_rest_response(
        &self,
        client_order_id: &str,
        exchange_order_id: u64,
    ) -> Option<ExecutionReport> {
        let mut result = None;

        if let Some(mut entry) = self.pending.get_mut(client_order_id) {
            match &*entry {
                PendingOrderState::AwaitingAck { .. } => {
                    // Normal case: REST arrived first
                    *entry = PendingOrderState::RestAcked { exchange_order_id };
                }
                PendingOrderState::WsArrivedFirst { execution_report } => {
                    // WS beat us - extract the report
                    result = Some(execution_report.clone());
                }
                PendingOrderState::RestAcked { .. } => {
                    // Duplicate REST response? Ignore
                    tracing::warn!(
                        client_order_id = %client_order_id,
                        "Duplicate REST response received"
                    );
                }
            }
        }

        // If WS arrived first, remove the entry since we're returning the report
        if result.is_some() {
            self.pending.remove(client_order_id);
        }

        result
    }

    /// Called when WebSocket executionReport arrives.
    ///
    /// Returns `true` if the REST response had already arrived (normal flow).
    /// Returns `false` if WS arrived first (will be stored for later retrieval).
    pub fn on_ws_execution_report(&self, execution_report: ExecutionReport) -> bool {
        let client_order_id = &execution_report.client_order_id;
        let mut rest_already_acked = false;

        self.pending
            .entry(client_order_id.clone())
            .and_modify(|state| {
                match state {
                    PendingOrderState::AwaitingAck { .. } => {
                        // WS arrived first - store the report
                        *state = PendingOrderState::WsArrivedFirst {
                            execution_report: execution_report.clone(),
                        };
                    }
                    PendingOrderState::RestAcked { .. } => {
                        // Normal case: REST already acked
                        rest_already_acked = true;
                    }
                    PendingOrderState::WsArrivedFirst { .. } => {
                        // Duplicate WS message - update with latest
                        *state = PendingOrderState::WsArrivedFirst {
                            execution_report: execution_report.clone(),
                        };
                    }
                }
            })
            .or_insert_with(|| {
                // Order not registered - this is a WS message for an order
                // we didn't track (maybe from a previous session or external)
                PendingOrderState::WsArrivedFirst {
                    execution_report: execution_report.clone(),
                }
            });

        rest_already_acked
    }

    /// Remove a completed order from the registry.
    ///
    /// Should be called when an order reaches a terminal state.
    pub fn remove(&self, client_order_id: &str) {
        self.pending.remove(client_order_id);
    }

    /// Clean up stale entries older than the given timeout.
    ///
    /// Call this periodically to prevent memory leaks from abandoned orders.
    pub fn cleanup_stale(&self, timeout_ms: i64, current_time_ms: i64) {
        self.pending.retain(|_, state| match state {
            PendingOrderState::AwaitingAck { created_at_ms } => {
                current_time_ms - *created_at_ms < timeout_ms
            }
            // Keep RestAcked and WsArrivedFirst entries
            // (they should be cleaned up by explicit remove calls)
            _ => true,
        });
    }

    /// Get the number of pending orders.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Shared handle to the pending order registry.
pub type SharedPendingOrderRegistry = Arc<PendingOrderRegistry>;

/// Create a new shared pending order registry.
pub fn create_pending_order_registry() -> SharedPendingOrderRegistry {
    Arc::new(PendingOrderRegistry::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderSide, OrderStatus, OrderType, TimeInForce};
    use rust_decimal_macros::dec;

    fn make_execution_report(client_order_id: &str) -> ExecutionReport {
        ExecutionReport {
            event_time_ms: 1000,
            symbol: "BTCUSDT".into(),
            client_order_id: client_order_id.into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::GTC,
            quantity: dec!(1.0),
            price: dec!(50000.0),
            order_status: OrderStatus::New,
            order_id: 12345,
            last_executed_qty: dec!(0),
            cumulative_filled_qty: dec!(0),
            last_executed_price: dec!(0),
            commission: dec!(0),
            commission_asset: "BNB".into(),
            trade_time_ms: 1000,
            trade_id: 0,
            is_maker: false,
        }
    }

    #[test]
    fn test_generate_client_order_id() {
        let id1 = generate_client_order_id("BOT");
        let id2 = generate_client_order_id("BOT");

        assert!(id1.starts_with("BOT_"));
        assert!(id2.starts_with("BOT_"));
        assert_ne!(id1, id2); // UUIDs should be unique
    }

    #[test]
    fn test_normal_flow_rest_first() {
        let registry = PendingOrderRegistry::new();
        let client_id = "test_order_1";

        // 1. Pre-register
        registry.register(client_id, 1000);
        assert_eq!(registry.len(), 1);

        // 2. REST response arrives first
        let ws_report = registry.on_rest_response(client_id, 12345);
        assert!(ws_report.is_none());

        // 3. WS arrives - should return true (REST already acked)
        let report = make_execution_report(client_id);
        let was_acked = registry.on_ws_execution_report(report);
        assert!(was_acked);
    }

    #[test]
    fn test_race_condition_ws_first() {
        let registry = PendingOrderRegistry::new();
        let client_id = "test_order_2";

        // 1. Pre-register
        registry.register(client_id, 1000);

        // 2. WS arrives first
        let report = make_execution_report(client_id);
        let was_acked = registry.on_ws_execution_report(report.clone());
        assert!(!was_acked);

        // 3. REST response arrives - should return the stored WS report
        let ws_report = registry.on_rest_response(client_id, 12345);
        assert!(ws_report.is_some());
        assert_eq!(ws_report.unwrap().client_order_id, client_id);

        // Registry should be cleaned up
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_unregistered_ws_message() {
        let registry = PendingOrderRegistry::new();
        let client_id = "unknown_order";

        // WS arrives for an order we didn't register
        let report = make_execution_report(client_id);
        let was_acked = registry.on_ws_execution_report(report);

        // Should return false (not acked, stored)
        assert!(!was_acked);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_cleanup_stale() {
        let registry = PendingOrderRegistry::new();

        registry.register("old_order", 1000);
        registry.register("new_order", 5000);

        // Cleanup with 2000ms timeout at time 6000
        registry.cleanup_stale(2000, 6000);

        // Only new_order should remain
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_remove() {
        let registry = PendingOrderRegistry::new();
        let client_id = "test_order";

        registry.register(client_id, 1000);
        assert_eq!(registry.len(), 1);

        registry.remove(client_id);
        assert_eq!(registry.len(), 0);
    }
}
