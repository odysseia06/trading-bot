//! Position tracking from execution reports.
//!
//! Tracks net positions per symbol and calculates PnL.

use crate::execution::ExecutionReport;
use crate::order::OrderSide;
use dashmap::DashMap;
use num_traits::Signed;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::sync::Arc;

/// A position in a single symbol.
#[derive(Debug, Clone)]
pub struct Position {
    /// Trading pair symbol.
    pub symbol: String,
    /// Net quantity (positive = long, negative = short).
    pub quantity: Decimal,
    /// Volume-weighted average entry price.
    pub avg_entry_price: Decimal,
    /// Realized PnL from closed trades.
    pub realized_pnl: Decimal,
    /// Total commission paid.
    pub total_commission: Decimal,
    /// Last update timestamp in milliseconds.
    pub last_update_ms: i64,
}

impl Position {
    /// Create a new empty position.
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            quantity: Decimal::ZERO,
            avg_entry_price: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            total_commission: Decimal::ZERO,
            last_update_ms: 0,
        }
    }

    /// Check if position is flat (no holdings).
    pub fn is_flat(&self) -> bool {
        self.quantity == Decimal::ZERO
    }

    /// Check if position is long.
    pub fn is_long(&self) -> bool {
        self.quantity > Decimal::ZERO
    }

    /// Check if position is short.
    pub fn is_short(&self) -> bool {
        self.quantity < Decimal::ZERO
    }

    /// Get absolute position size.
    pub fn abs_quantity(&self) -> Decimal {
        self.quantity.abs()
    }

    /// Calculate unrealized PnL at a given price.
    pub fn unrealized_pnl(&self, current_price: Decimal) -> Decimal {
        if self.is_flat() {
            Decimal::ZERO
        } else {
            (current_price - self.avg_entry_price) * self.quantity
        }
    }

    /// Calculate notional value at a given price.
    pub fn notional_value(&self, current_price: Decimal) -> Decimal {
        self.abs_quantity() * current_price
    }

    /// Apply a fill to this position.
    ///
    /// Updates quantity, average entry price, and realized PnL.
    pub fn apply_fill(
        &mut self,
        side: OrderSide,
        fill_qty: Decimal,
        fill_price: Decimal,
        commission: Decimal,
        timestamp_ms: i64,
    ) {
        if fill_qty == Decimal::ZERO {
            return;
        }

        self.total_commission += commission;
        self.last_update_ms = timestamp_ms;

        // Determine signed quantity change
        let qty_delta = match side {
            OrderSide::Buy => fill_qty,
            OrderSide::Sell => -fill_qty,
        };

        let old_qty = self.quantity;
        let new_qty = old_qty + qty_delta;

        // Calculate realized PnL and new average entry price
        if old_qty == Decimal::ZERO {
            // Opening a new position
            self.avg_entry_price = fill_price;
        } else if (old_qty > Decimal::ZERO && qty_delta > Decimal::ZERO)
            || (old_qty < Decimal::ZERO && qty_delta < Decimal::ZERO)
        {
            // Adding to existing position - update weighted average
            let old_cost = old_qty.abs() * self.avg_entry_price;
            let new_cost = fill_qty * fill_price;
            let total_qty = old_qty.abs() + fill_qty;
            if total_qty != Decimal::ZERO {
                self.avg_entry_price = (old_cost + new_cost) / total_qty;
            }
        } else {
            // Reducing or reversing position
            let closed_qty = fill_qty.min(old_qty.abs());

            // Realized PnL from the closed portion
            if old_qty > Decimal::ZERO {
                // Was long, selling
                self.realized_pnl += (fill_price - self.avg_entry_price) * closed_qty;
            } else {
                // Was short, buying
                self.realized_pnl += (self.avg_entry_price - fill_price) * closed_qty;
            }

            // If position reversed, set new entry price for the excess
            if new_qty != Decimal::ZERO && new_qty.signum() != old_qty.signum() {
                self.avg_entry_price = fill_price;
            }
        }

        self.quantity = new_qty;

        // Clear entry price if flat
        if self.is_flat() {
            self.avg_entry_price = Decimal::ZERO;
        }
    }
}

/// Thread-safe position tracker across all symbols.
pub struct PositionTracker {
    positions: DashMap<String, Position>,
    total_realized_pnl: RwLock<Decimal>,
}

impl PositionTracker {
    /// Create a new position tracker.
    pub fn new() -> Self {
        Self {
            positions: DashMap::new(),
            total_realized_pnl: RwLock::new(Decimal::ZERO),
        }
    }

    /// Update position from an execution report.
    ///
    /// Only updates if the report contains a fill (last_executed_qty > 0).
    pub fn update_from_fill(&self, report: &ExecutionReport) {
        if !report.has_fill() {
            return;
        }

        let mut entry = self
            .positions
            .entry(report.symbol.clone())
            .or_insert_with(|| Position::new(&report.symbol));

        let old_realized = entry.realized_pnl;

        entry.apply_fill(
            report.side,
            report.last_executed_qty,
            report.last_executed_price,
            report.commission,
            report.event_time_ms,
        );

        // Update total realized PnL
        let pnl_delta = entry.realized_pnl - old_realized;
        if pnl_delta != Decimal::ZERO {
            let mut total = self.total_realized_pnl.write();
            *total += pnl_delta;
        }
    }

    /// Get position for a symbol.
    pub fn get_position(&self, symbol: &str) -> Option<Position> {
        self.positions.get(symbol).map(|p| p.clone())
    }

    /// Get all positions.
    pub fn get_all_positions(&self) -> Vec<Position> {
        self.positions.iter().map(|p| p.clone()).collect()
    }

    /// Get net quantity for a symbol (0 if no position).
    pub fn net_quantity(&self, symbol: &str) -> Decimal {
        self.positions
            .get(symbol)
            .map(|p| p.quantity)
            .unwrap_or(Decimal::ZERO)
    }

    /// Get total realized PnL across all symbols.
    pub fn total_realized_pnl(&self) -> Decimal {
        *self.total_realized_pnl.read()
    }

    /// Calculate total unrealized PnL given current prices.
    pub fn total_unrealized_pnl<F>(&self, price_fn: F) -> Decimal
    where
        F: Fn(&str) -> Option<Decimal>,
    {
        self.positions
            .iter()
            .filter_map(|entry| {
                let pos = entry.value();
                price_fn(&pos.symbol).map(|price| pos.unrealized_pnl(price))
            })
            .sum()
    }

    /// Calculate total exposure (sum of absolute notional values).
    pub fn total_exposure<F>(&self, price_fn: F) -> Decimal
    where
        F: Fn(&str) -> Option<Decimal>,
    {
        self.positions
            .iter()
            .filter_map(|entry| {
                let pos = entry.value();
                price_fn(&pos.symbol).map(|price| pos.notional_value(price))
            })
            .sum()
    }

    /// Get number of open positions.
    pub fn open_position_count(&self) -> usize {
        self.positions.iter().filter(|p| !p.is_flat()).count()
    }

    /// Reset all positions and PnL (for testing or daily reset).
    pub fn reset(&self) {
        self.positions.clear();
        *self.total_realized_pnl.write() = Decimal::ZERO;
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared position tracker type.
pub type SharedPositionTracker = Arc<PositionTracker>;

/// Create a new shared position tracker.
pub fn create_position_tracker() -> SharedPositionTracker {
    Arc::new(PositionTracker::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderStatus, OrderType, TimeInForce};
    use rust_decimal_macros::dec;

    fn make_fill_report(
        symbol: &str,
        side: OrderSide,
        qty: Decimal,
        price: Decimal,
    ) -> ExecutionReport {
        ExecutionReport {
            event_time_ms: 1000,
            symbol: symbol.to_string(),
            client_order_id: "test".to_string(),
            side,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::GTC,
            quantity: qty,
            price,
            order_status: OrderStatus::Filled,
            order_id: 1,
            last_executed_qty: qty,
            cumulative_filled_qty: qty,
            last_executed_price: price,
            commission: dec!(0.001),
            commission_asset: "BNB".to_string(),
            trade_time_ms: 1000,
            trade_id: 1,
            is_maker: false,
        }
    }

    #[test]
    fn test_position_new() {
        let pos = Position::new("BTCUSDT");
        assert_eq!(pos.symbol, "BTCUSDT");
        assert!(pos.is_flat());
        assert!(!pos.is_long());
        assert!(!pos.is_short());
    }

    #[test]
    fn test_position_buy() {
        let mut pos = Position::new("BTCUSDT");
        pos.apply_fill(OrderSide::Buy, dec!(1), dec!(50000), dec!(0), 1000);

        assert!(pos.is_long());
        assert_eq!(pos.quantity, dec!(1));
        assert_eq!(pos.avg_entry_price, dec!(50000));
        assert_eq!(pos.realized_pnl, dec!(0));
    }

    #[test]
    fn test_position_sell_to_flat() {
        let mut pos = Position::new("BTCUSDT");
        pos.apply_fill(OrderSide::Buy, dec!(1), dec!(50000), dec!(0), 1000);
        pos.apply_fill(OrderSide::Sell, dec!(1), dec!(51000), dec!(0), 2000);

        assert!(pos.is_flat());
        assert_eq!(pos.realized_pnl, dec!(1000)); // Profit of $1000
    }

    #[test]
    fn test_position_partial_close() {
        let mut pos = Position::new("BTCUSDT");
        pos.apply_fill(OrderSide::Buy, dec!(2), dec!(50000), dec!(0), 1000);
        pos.apply_fill(OrderSide::Sell, dec!(1), dec!(52000), dec!(0), 2000);

        assert!(pos.is_long());
        assert_eq!(pos.quantity, dec!(1));
        assert_eq!(pos.avg_entry_price, dec!(50000)); // Avg unchanged for remaining
        assert_eq!(pos.realized_pnl, dec!(2000)); // Profit of $2000 on 1 BTC
    }

    #[test]
    fn test_position_add_to_long() {
        let mut pos = Position::new("BTCUSDT");
        pos.apply_fill(OrderSide::Buy, dec!(1), dec!(50000), dec!(0), 1000);
        pos.apply_fill(OrderSide::Buy, dec!(1), dec!(52000), dec!(0), 2000);

        assert_eq!(pos.quantity, dec!(2));
        assert_eq!(pos.avg_entry_price, dec!(51000)); // Weighted average
    }

    #[test]
    fn test_position_unrealized_pnl() {
        let mut pos = Position::new("BTCUSDT");
        pos.apply_fill(OrderSide::Buy, dec!(1), dec!(50000), dec!(0), 1000);

        assert_eq!(pos.unrealized_pnl(dec!(51000)), dec!(1000));
        assert_eq!(pos.unrealized_pnl(dec!(49000)), dec!(-1000));
    }

    #[test]
    fn test_position_tracker_update() {
        let tracker = PositionTracker::new();
        let report = make_fill_report("BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000));

        tracker.update_from_fill(&report);

        let pos = tracker.get_position("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, dec!(1));
        assert_eq!(pos.avg_entry_price, dec!(50000));
    }

    #[test]
    fn test_position_tracker_pnl() {
        let tracker = PositionTracker::new();

        // Buy 1 BTC at $50000
        tracker.update_from_fill(&make_fill_report(
            "BTCUSDT",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));

        // Sell 1 BTC at $51000
        tracker.update_from_fill(&make_fill_report(
            "BTCUSDT",
            OrderSide::Sell,
            dec!(1),
            dec!(51000),
        ));

        assert_eq!(tracker.total_realized_pnl(), dec!(1000));
    }

    #[test]
    fn test_position_tracker_exposure() {
        let tracker = PositionTracker::new();

        tracker.update_from_fill(&make_fill_report(
            "BTCUSDT",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));
        tracker.update_from_fill(&make_fill_report(
            "ETHUSDT",
            OrderSide::Buy,
            dec!(10),
            dec!(3000),
        ));

        let exposure = tracker.total_exposure(|symbol| match symbol {
            "BTCUSDT" => Some(dec!(50000)),
            "ETHUSDT" => Some(dec!(3000)),
            _ => None,
        });

        // 1 BTC * $50000 + 10 ETH * $3000 = $80000
        assert_eq!(exposure, dec!(80000));
    }

    #[test]
    fn test_position_short() {
        let mut pos = Position::new("BTCUSDT");
        pos.apply_fill(OrderSide::Sell, dec!(1), dec!(50000), dec!(0), 1000);

        assert!(pos.is_short());
        assert_eq!(pos.quantity, dec!(-1));
        assert_eq!(pos.avg_entry_price, dec!(50000));

        // Close short with profit (price went down)
        pos.apply_fill(OrderSide::Buy, dec!(1), dec!(49000), dec!(0), 2000);

        assert!(pos.is_flat());
        assert_eq!(pos.realized_pnl, dec!(1000)); // Profit from short
    }
}
