//! Execution report from exchange.

use crate::order::{OrderSide, OrderStatus, OrderType, TimeInForce};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Execution report from exchange (maps to Binance executionReport).
///
/// This is received via WebSocket when an order status changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    /// Event timestamp in milliseconds.
    pub event_time_ms: i64,
    /// Trading pair symbol.
    pub symbol: String,
    /// Client-generated order ID.
    pub client_order_id: String,
    /// Order side (buy/sell).
    pub side: OrderSide,
    /// Order type.
    pub order_type: OrderType,
    /// Time in force.
    pub time_in_force: TimeInForce,
    /// Original order quantity.
    pub quantity: Decimal,
    /// Order price.
    pub price: Decimal,
    /// Current order status.
    pub order_status: OrderStatus,
    /// Exchange-assigned order ID.
    pub order_id: u64,
    /// Quantity of the last executed trade.
    pub last_executed_qty: Decimal,
    /// Cumulative filled quantity.
    pub cumulative_filled_qty: Decimal,
    /// Price of the last executed trade.
    pub last_executed_price: Decimal,
    /// Commission paid.
    pub commission: Decimal,
    /// Asset used for commission.
    pub commission_asset: String,
    /// Trade timestamp in milliseconds.
    pub trade_time_ms: i64,
    /// Trade ID (0 if no trade occurred).
    pub trade_id: i64,
    /// Whether this side was the maker.
    pub is_maker: bool,
}

impl ExecutionReport {
    /// Check if this report indicates the order is done (terminal state).
    pub fn is_terminal(&self) -> bool {
        self.order_status.is_terminal()
    }

    /// Check if this report indicates a fill occurred.
    pub fn has_fill(&self) -> bool {
        self.last_executed_qty > Decimal::ZERO
    }

    /// Calculate remaining quantity to be filled.
    pub fn remaining_qty(&self) -> Decimal {
        self.quantity - self.cumulative_filled_qty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_report() -> ExecutionReport {
        ExecutionReport {
            event_time_ms: 1000,
            symbol: "BTCUSDT".into(),
            client_order_id: "test_order".into(),
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
    fn test_is_terminal() {
        let mut report = make_report();
        assert!(!report.is_terminal());

        report.order_status = OrderStatus::Filled;
        assert!(report.is_terminal());

        report.order_status = OrderStatus::Canceled;
        assert!(report.is_terminal());
    }

    #[test]
    fn test_has_fill() {
        let mut report = make_report();
        assert!(!report.has_fill());

        report.last_executed_qty = dec!(0.1);
        assert!(report.has_fill());
    }

    #[test]
    fn test_remaining_qty() {
        let mut report = make_report();
        assert_eq!(report.remaining_qty(), dec!(1.0));

        report.cumulative_filled_qty = dec!(0.3);
        assert_eq!(report.remaining_qty(), dec!(0.7));
    }
}
