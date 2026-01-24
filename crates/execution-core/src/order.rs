//! Order types and status enums.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Order side (buy or sell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    /// Convert from Binance string representation.
    pub fn from_binance_str(s: &str) -> Option<Self> {
        match s {
            "BUY" => Some(Self::Buy),
            "SELL" => Some(Self::Sell),
            _ => None,
        }
    }

    /// Convert to Binance string representation.
    pub fn as_binance_str(&self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }
}

/// Order type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    StopLoss,
    StopLossLimit,
    TakeProfit,
    TakeProfitLimit,
    LimitMaker,
}

impl OrderType {
    /// Convert from Binance string representation.
    pub fn from_binance_str(s: &str) -> Option<Self> {
        match s {
            "MARKET" => Some(Self::Market),
            "LIMIT" => Some(Self::Limit),
            "STOP_LOSS" => Some(Self::StopLoss),
            "STOP_LOSS_LIMIT" => Some(Self::StopLossLimit),
            "TAKE_PROFIT" => Some(Self::TakeProfit),
            "TAKE_PROFIT_LIMIT" => Some(Self::TakeProfitLimit),
            "LIMIT_MAKER" => Some(Self::LimitMaker),
            _ => None,
        }
    }

    /// Convert to Binance string representation.
    pub fn as_binance_str(&self) -> &'static str {
        match self {
            Self::Market => "MARKET",
            Self::Limit => "LIMIT",
            Self::StopLoss => "STOP_LOSS",
            Self::StopLossLimit => "STOP_LOSS_LIMIT",
            Self::TakeProfit => "TAKE_PROFIT",
            Self::TakeProfitLimit => "TAKE_PROFIT_LIMIT",
            Self::LimitMaker => "LIMIT_MAKER",
        }
    }
}

/// Order status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// Order submitted, awaiting acknowledgment from exchange.
    PendingAck,
    /// Order acknowledged by exchange.
    New,
    /// Order partially filled.
    PartiallyFilled,
    /// Order completely filled.
    Filled,
    /// Order canceled by user.
    Canceled,
    /// Order rejected by exchange.
    Rejected,
    /// Order expired (e.g., GTT orders).
    Expired,
    /// Order pending cancellation.
    PendingCancel,
}

impl OrderStatus {
    /// Convert from Binance string representation.
    pub fn from_binance_str(s: &str) -> Option<Self> {
        match s {
            "NEW" => Some(Self::New),
            "PARTIALLY_FILLED" => Some(Self::PartiallyFilled),
            "FILLED" => Some(Self::Filled),
            "CANCELED" => Some(Self::Canceled),
            "REJECTED" => Some(Self::Rejected),
            "EXPIRED" => Some(Self::Expired),
            "PENDING_CANCEL" => Some(Self::PendingCancel),
            _ => None,
        }
    }

    /// Check if this is a terminal status (order is done).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Canceled | Self::Rejected | Self::Expired
        )
    }

    /// Check if the order is still active/open.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::PendingAck | Self::New | Self::PartiallyFilled | Self::PendingCancel
        )
    }
}

/// Time in force for limit orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Good Till Canceled - remains active until filled or canceled.
    GTC,
    /// Immediate Or Cancel - fill what's possible immediately, cancel rest.
    IOC,
    /// Fill Or Kill - must be filled completely immediately or canceled.
    FOK,
}

impl TimeInForce {
    /// Convert from Binance string representation.
    pub fn from_binance_str(s: &str) -> Option<Self> {
        match s {
            "GTC" => Some(Self::GTC),
            "IOC" => Some(Self::IOC),
            "FOK" => Some(Self::FOK),
            _ => None,
        }
    }

    /// Convert to Binance string representation.
    pub fn as_binance_str(&self) -> &'static str {
        match self {
            Self::GTC => "GTC",
            Self::IOC => "IOC",
            Self::FOK => "FOK",
        }
    }
}

/// An order tracked by the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    /// Client-generated order ID (used for correlation).
    pub client_order_id: String,
    /// Exchange-assigned order ID (populated after acknowledgment).
    pub exchange_order_id: Option<u64>,
    /// Trading pair symbol (e.g., "BTCUSDT").
    pub symbol: String,
    /// Buy or sell.
    pub side: OrderSide,
    /// Order type (market, limit, etc.).
    pub order_type: OrderType,
    /// Requested quantity.
    pub quantity: Decimal,
    /// Limit price (None for market orders).
    pub price: Option<Decimal>,
    /// Time in force (for limit orders).
    pub time_in_force: Option<TimeInForce>,
    /// Current order status.
    pub status: OrderStatus,
    /// Quantity filled so far.
    pub filled_qty: Decimal,
    /// Average fill price (None if no fills yet).
    pub avg_fill_price: Option<Decimal>,
    /// Timestamp when order was created locally.
    pub created_at_ms: i64,
    /// Timestamp when order was last updated.
    pub updated_at_ms: i64,
}

impl Order {
    /// Create a new order in PendingAck status.
    pub fn new(
        client_order_id: String,
        symbol: String,
        side: OrderSide,
        order_type: OrderType,
        quantity: Decimal,
        price: Option<Decimal>,
        time_in_force: Option<TimeInForce>,
        created_at_ms: i64,
    ) -> Self {
        Self {
            client_order_id,
            exchange_order_id: None,
            symbol,
            side,
            order_type,
            quantity,
            price,
            time_in_force,
            status: OrderStatus::PendingAck,
            filled_qty: Decimal::ZERO,
            avg_fill_price: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        }
    }

    /// Calculate remaining quantity to be filled.
    pub fn remaining_qty(&self) -> Decimal {
        self.quantity - self.filled_qty
    }

    /// Check if the order is completely filled.
    pub fn is_filled(&self) -> bool {
        self.status == OrderStatus::Filled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_order_side_conversion() {
        assert_eq!(OrderSide::from_binance_str("BUY"), Some(OrderSide::Buy));
        assert_eq!(OrderSide::from_binance_str("SELL"), Some(OrderSide::Sell));
        assert_eq!(OrderSide::from_binance_str("INVALID"), None);

        assert_eq!(OrderSide::Buy.as_binance_str(), "BUY");
        assert_eq!(OrderSide::Sell.as_binance_str(), "SELL");
    }

    #[test]
    fn test_order_status_terminal() {
        assert!(OrderStatus::Filled.is_terminal());
        assert!(OrderStatus::Canceled.is_terminal());
        assert!(OrderStatus::Rejected.is_terminal());
        assert!(OrderStatus::Expired.is_terminal());

        assert!(!OrderStatus::New.is_terminal());
        assert!(!OrderStatus::PartiallyFilled.is_terminal());
        assert!(!OrderStatus::PendingAck.is_terminal());
    }

    #[test]
    fn test_order_status_active() {
        assert!(OrderStatus::New.is_active());
        assert!(OrderStatus::PartiallyFilled.is_active());
        assert!(OrderStatus::PendingAck.is_active());

        assert!(!OrderStatus::Filled.is_active());
        assert!(!OrderStatus::Canceled.is_active());
    }

    #[test]
    fn test_order_remaining_qty() {
        let mut order = Order::new(
            "test".into(),
            "BTCUSDT".into(),
            OrderSide::Buy,
            OrderType::Limit,
            dec!(1.0),
            Some(dec!(50000.0)),
            Some(TimeInForce::GTC),
            1000,
        );

        assert_eq!(order.remaining_qty(), dec!(1.0));

        order.filled_qty = dec!(0.3);
        assert_eq!(order.remaining_qty(), dec!(0.7));

        order.filled_qty = dec!(1.0);
        assert_eq!(order.remaining_qty(), dec!(0.0));
    }
}
