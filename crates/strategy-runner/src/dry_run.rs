//! Dry run executor for simulated order fills.

use std::sync::atomic::{AtomicU64, Ordering};

use rust_decimal::Decimal;

use execution_core::{ExecutionReport, OrderStatus, OrderType, TimeInForce};
use strategy_core::{OrderIntent, OrderSide};

/// Counter for generating unique simulated order IDs.
static SIMULATED_ORDER_ID: AtomicU64 = AtomicU64::new(1_000_000);
/// Counter for generating unique simulated trade IDs.
static SIMULATED_TRADE_ID: AtomicU64 = AtomicU64::new(1_000_000);

/// Generates simulated execution reports for dry run mode.
///
/// This allows strategies to track their simulated positions and P&L
/// without sending real orders to the exchange.
pub struct DryRunExecutor;

impl DryRunExecutor {
    /// Create a simulated fill execution report for an order.
    ///
    /// Assumes immediate full fill at the specified price (or market price for market orders).
    pub fn simulate_fill(
        intent: &OrderIntent,
        client_order_id: &str,
        current_price: Option<Decimal>,
        timestamp_ms: i64,
    ) -> ExecutionReport {
        // Determine the execution price
        let exec_price = match intent.order_type {
            // Market orders execute at current price
            OrderType::Market => current_price.unwrap_or(Decimal::ZERO),
            // Limit orders execute at limit price (assuming fill)
            OrderType::Limit | OrderType::LimitMaker => intent.price.unwrap_or(Decimal::ZERO),
            // Stop orders execute at stop price (simplified)
            OrderType::StopLoss | OrderType::TakeProfit => {
                intent.stop_price.unwrap_or(Decimal::ZERO)
            }
            // Stop-limit orders execute at limit price
            OrderType::StopLossLimit | OrderType::TakeProfitLimit => {
                intent.price.unwrap_or(Decimal::ZERO)
            }
        };

        // Generate unique IDs for this simulated fill
        let order_id = SIMULATED_ORDER_ID.fetch_add(1, Ordering::Relaxed);
        let trade_id = SIMULATED_TRADE_ID.fetch_add(1, Ordering::Relaxed);

        // Simulate a small commission (0.1% of notional)
        let notional = exec_price * intent.quantity;
        let commission = notional * Decimal::new(1, 3); // 0.001 = 0.1%

        ExecutionReport {
            event_time_ms: timestamp_ms,
            symbol: intent.symbol.clone(),
            client_order_id: client_order_id.to_string(),
            side: convert_order_side(intent.side),
            order_type: intent.order_type,
            time_in_force: intent.time_in_force.unwrap_or(TimeInForce::GTC),
            quantity: intent.quantity,
            price: intent.price.unwrap_or(exec_price),
            order_status: OrderStatus::Filled,
            order_id,
            last_executed_qty: intent.quantity,
            cumulative_filled_qty: intent.quantity,
            last_executed_price: exec_price,
            commission,
            commission_asset: "USDT".to_string(), // Assume USDT for simulation
            trade_time_ms: timestamp_ms,
            trade_id: trade_id as i64,
            is_maker: matches!(intent.order_type, OrderType::Limit | OrderType::LimitMaker),
        }
    }
}

/// Convert strategy OrderSide to execution OrderSide.
fn convert_order_side(side: OrderSide) -> execution_core::OrderSide {
    match side {
        OrderSide::Buy => execution_core::OrderSide::Buy,
        OrderSide::Sell => execution_core::OrderSide::Sell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_simulate_market_buy() {
        let intent = OrderIntent::market_buy("BTCUSDT", dec!(0.1));
        let current_price = Some(dec!(50000));

        let report = DryRunExecutor::simulate_fill(&intent, "test_order_1", current_price, 1000);

        assert_eq!(report.symbol, "BTCUSDT");
        assert_eq!(report.client_order_id, "test_order_1");
        assert_eq!(report.side, execution_core::OrderSide::Buy);
        assert_eq!(report.order_type, OrderType::Market);
        assert_eq!(report.quantity, dec!(0.1));
        assert_eq!(report.last_executed_price, dec!(50000));
        assert_eq!(report.cumulative_filled_qty, dec!(0.1));
        assert_eq!(report.order_status, OrderStatus::Filled);
        assert!(!report.is_maker); // Market orders are takers
    }

    #[test]
    fn test_simulate_limit_sell() {
        let intent = OrderIntent::limit_sell("ETHUSDT", dec!(1.5), dec!(3000));

        let report = DryRunExecutor::simulate_fill(&intent, "test_order_2", None, 2000);

        assert_eq!(report.symbol, "ETHUSDT");
        assert_eq!(report.side, execution_core::OrderSide::Sell);
        assert_eq!(report.order_type, OrderType::Limit);
        assert_eq!(report.last_executed_price, dec!(3000));
        assert_eq!(report.price, dec!(3000));
        assert!(report.is_maker); // Limit orders are makers
    }

    #[test]
    fn test_simulate_commission() {
        let intent = OrderIntent::limit_buy("BTCUSDT", dec!(1.0), dec!(50000));

        let report = DryRunExecutor::simulate_fill(&intent, "test_order_3", None, 3000);

        // Commission should be 0.1% of notional (50000 * 1.0 = 50000)
        // 50000 * 0.001 = 50
        assert_eq!(report.commission, dec!(50));
        assert_eq!(report.commission_asset, "USDT");
    }

    #[test]
    fn test_unique_ids() {
        let intent = OrderIntent::market_buy("BTCUSDT", dec!(0.1));

        let report1 = DryRunExecutor::simulate_fill(&intent, "order1", Some(dec!(50000)), 1000);
        let report2 = DryRunExecutor::simulate_fill(&intent, "order2", Some(dec!(50000)), 1001);

        assert_ne!(report1.order_id, report2.order_id);
        assert_ne!(report1.trade_id, report2.trade_id);
    }
}
