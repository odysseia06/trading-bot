//! Risk manager for pre-trade checks and loss protection.
//!
//! The risk manager sits between the signal processor and order execution,
//! enforcing position limits, loss limits, and circuit breakers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::RwLock;
use rust_decimal::Decimal;
use tracing::{info, warn};

use execution_core::{ExecutionReport, SharedPendingOrderRegistry, SharedPositionTracker};
use strategy_core::OrderIntent;

use crate::error::RiskRejection;
use crate::risk_config::RiskConfig;

/// Result of a risk check on an order.
#[derive(Debug, Clone)]
pub enum RiskCheckResult {
    /// Order approved - may proceed to execution.
    Approved,
    /// Order rejected - must not be executed.
    Rejected(RiskRejection),
}

/// Actions the risk manager may request in response to risk events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskAction {
    /// Trigger the circuit breaker (pause trading).
    TriggerCircuitBreaker,
    /// Cancel all open orders.
    CancelAllOrders,
    /// Close all positions (flatten).
    CloseAllPositions,
}

/// Current status of the risk manager.
#[derive(Debug, Clone)]
pub struct RiskStatus {
    /// Whether the kill switch is active.
    pub is_killed: bool,
    /// Whether the circuit breaker is active.
    pub circuit_breaker_active: bool,
    /// Timestamp when circuit breaker lifts (if active).
    pub circuit_breaker_until_ms: Option<i64>,
    /// Current daily realized PnL.
    pub daily_pnl: Decimal,
    /// Current drawdown percentage.
    pub current_drawdown_pct: Decimal,
    /// Peak equity since session start.
    pub peak_equity: Decimal,
    /// Current total exposure across all symbols.
    pub total_exposure: Decimal,
    /// Number of open orders.
    pub open_order_count: u32,
}

/// Risk manager that enforces trading limits and circuit breakers.
///
/// # Thread Safety
///
/// The risk manager uses interior mutability for thread-safe access:
/// - `is_killed` uses an atomic boolean for lock-free reads
/// - Other mutable state is protected by RwLock
///
/// # Usage
///
/// ```rust,ignore
/// let risk_manager = RiskManager::new(config, position_tracker, pending_orders);
///
/// // Before placing an order
/// match risk_manager.check_order(&intent, current_price) {
///     RiskCheckResult::Approved => { /* proceed */ }
///     RiskCheckResult::Rejected(reason) => { /* reject with reason */ }
/// }
///
/// // After a fill
/// risk_manager.on_fill(&execution_report);
///
/// // Periodic check (e.g., every second)
/// if let Some(action) = risk_manager.periodic_check(&prices, current_time_ms) {
///     match action {
///         RiskAction::TriggerCircuitBreaker => { /* pause trading */ }
///         RiskAction::CancelAllOrders => { /* cancel orders */ }
///         RiskAction::CloseAllPositions => { /* flatten */ }
///     }
/// }
/// ```
pub struct RiskManager {
    config: RiskConfig,
    positions: SharedPositionTracker,
    pending_orders: SharedPendingOrderRegistry,

    // Mutable state protected by RwLock
    state: RwLock<RiskManagerState>,

    // Kill switch uses atomic for fast lock-free checks
    is_killed: AtomicBool,
}

/// Internal mutable state of the risk manager.
struct RiskManagerState {
    /// Daily realized PnL (reset at start of trading day).
    daily_pnl: Decimal,
    /// Peak equity since session start (for drawdown calculation).
    peak_equity: Decimal,
    /// Current equity estimate.
    current_equity: Decimal,
    /// Timestamp when circuit breaker lifts (None = not active).
    circuit_breaker_until_ms: Option<i64>,
    /// Timestamp when trading session started.
    trading_start_ms: i64,
}

impl RiskManager {
    /// Create a new risk manager.
    pub fn new(
        config: RiskConfig,
        positions: SharedPositionTracker,
        pending_orders: SharedPendingOrderRegistry,
    ) -> Self {
        let now_ms = chrono::Utc::now().timestamp_millis();

        Self {
            is_killed: AtomicBool::new(config.enable_kill_switch),
            config,
            positions,
            pending_orders,
            state: RwLock::new(RiskManagerState {
                daily_pnl: Decimal::ZERO,
                peak_equity: Decimal::ZERO,
                current_equity: Decimal::ZERO,
                circuit_breaker_until_ms: None,
                trading_start_ms: now_ms,
            }),
        }
    }

    /// Check if an order should be allowed.
    ///
    /// This performs all pre-trade risk checks in order of severity:
    /// 1. Kill switch
    /// 2. Circuit breaker
    /// 3. Daily loss limit
    /// 4. Drawdown limit
    /// 5. Position limits
    /// 6. Exposure limits
    /// 7. Open order limits
    /// 8. Order size limits
    pub fn check_order(&self, intent: &OrderIntent, current_price: Decimal) -> RiskCheckResult {
        // 1. Kill switch - immediate rejection
        if self.is_killed.load(Ordering::Relaxed) {
            return RiskCheckResult::Rejected(RiskRejection::KillSwitchActive);
        }

        let state = self.state.read();
        let now_ms = chrono::Utc::now().timestamp_millis();

        // 2. Circuit breaker
        if let Some(resumes_at_ms) = state.circuit_breaker_until_ms {
            if now_ms < resumes_at_ms {
                return RiskCheckResult::Rejected(RiskRejection::CircuitBreakerActive {
                    resumes_at_ms,
                });
            }
        }

        // 3. Daily loss limit
        if state.daily_pnl < Decimal::ZERO && state.daily_pnl.abs() >= self.config.max_daily_loss {
            return RiskCheckResult::Rejected(RiskRejection::DailyLossLimitExceeded {
                current: state.daily_pnl,
                limit: self.config.max_daily_loss,
            });
        }

        // 4. Drawdown limit
        if state.peak_equity > Decimal::ZERO {
            let drawdown = state.peak_equity - state.current_equity;
            let drawdown_pct = (drawdown / state.peak_equity) * Decimal::from(100);
            if drawdown_pct >= self.config.max_drawdown_pct {
                return RiskCheckResult::Rejected(RiskRejection::DrawdownLimitExceeded {
                    current_pct: drawdown_pct,
                    limit_pct: self.config.max_drawdown_pct,
                });
            }
        }

        // Calculate order notional
        let order_notional = current_price * intent.quantity;

        // 5. Position limit for this symbol
        let current_position = self.positions.get_position(&intent.symbol);
        let current_position_notional = current_position
            .as_ref()
            .map(|p| p.quantity.abs() * current_price)
            .unwrap_or(Decimal::ZERO);

        // Calculate new position after this order
        let position_delta = match intent.side {
            strategy_core::OrderSide::Buy => intent.quantity,
            strategy_core::OrderSide::Sell => -intent.quantity,
        };
        let current_qty = current_position
            .as_ref()
            .map(|p| p.quantity)
            .unwrap_or(Decimal::ZERO);
        let new_position_qty = current_qty + position_delta;
        let new_position_notional = new_position_qty.abs() * current_price;

        if new_position_notional > self.config.max_position_notional {
            return RiskCheckResult::Rejected(RiskRejection::PositionLimitExceeded {
                symbol: intent.symbol.clone(),
                current: current_position_notional,
                limit: self.config.max_position_notional,
            });
        }

        // 6. Total exposure limit
        let price_fn = |_symbol: &str| Some(current_price); // Simplified - use provided price
        let current_exposure = self.positions.total_exposure(price_fn);
        // Estimate new exposure (simplified - assumes adding to exposure)
        let exposure_change = order_notional;
        let new_exposure = current_exposure + exposure_change;

        if new_exposure > self.config.max_total_exposure {
            return RiskCheckResult::Rejected(RiskRejection::TotalExposureLimitExceeded {
                current: current_exposure,
                limit: self.config.max_total_exposure,
            });
        }

        // 7. Open order limits
        let total_open_orders = self.pending_orders.len() as u32;
        if total_open_orders >= self.config.max_open_orders_total {
            return RiskCheckResult::Rejected(RiskRejection::TooManyOpenOrdersTotal {
                current: total_open_orders,
                limit: self.config.max_open_orders_total,
            });
        }

        // 8. Single order size limit
        if order_notional > self.config.max_order_notional {
            return RiskCheckResult::Rejected(RiskRejection::OrderTooLarge {
                notional: order_notional,
                limit: self.config.max_order_notional,
            });
        }

        RiskCheckResult::Approved
    }

    /// Update risk state after a fill.
    ///
    /// This updates the daily PnL and equity tracking based on execution reports.
    pub fn on_fill(&self, report: &ExecutionReport) {
        if !report.has_fill() {
            return;
        }

        let mut state = self.state.write();

        // Get position's realized PnL from tracker
        let realized_pnl = self.positions.total_realized_pnl();
        state.daily_pnl = realized_pnl;

        // Update equity estimate (simplified - just tracks realized PnL)
        state.current_equity = realized_pnl;

        // Update peak equity
        if state.current_equity > state.peak_equity {
            state.peak_equity = state.current_equity;
        }

        info!(
            symbol = %report.symbol,
            fill_qty = %report.last_executed_qty,
            fill_price = %report.last_executed_price,
            daily_pnl = %state.daily_pnl,
            "risk manager: fill processed"
        );
    }

    /// Perform periodic risk checks.
    ///
    /// Call this regularly (e.g., every second) to check for risk events
    /// that should trigger protective actions.
    ///
    /// Returns an action if one should be taken.
    pub fn periodic_check(
        &self,
        _current_prices: &HashMap<String, Decimal>,
        current_time_ms: i64,
    ) -> Option<RiskAction> {
        // Check if circuit breaker has expired
        {
            let mut state = self.state.write();
            if let Some(resumes_at_ms) = state.circuit_breaker_until_ms {
                if current_time_ms >= resumes_at_ms {
                    info!("circuit breaker lifted");
                    state.circuit_breaker_until_ms = None;
                }
            }
        }

        let state = self.state.read();

        // Check if we should trigger circuit breaker due to daily loss
        if state.daily_pnl < Decimal::ZERO
            && state.daily_pnl.abs() >= self.config.max_daily_loss
            && state.circuit_breaker_until_ms.is_none()
        {
            drop(state);
            self.trigger_circuit_breaker();
            return Some(RiskAction::TriggerCircuitBreaker);
        }

        // Check drawdown
        if state.peak_equity > Decimal::ZERO {
            let drawdown = state.peak_equity - state.current_equity;
            let drawdown_pct = (drawdown / state.peak_equity) * Decimal::from(100);
            if drawdown_pct >= self.config.max_drawdown_pct
                && state.circuit_breaker_until_ms.is_none()
            {
                drop(state);
                self.trigger_circuit_breaker();
                return Some(RiskAction::TriggerCircuitBreaker);
            }
        }

        None
    }

    /// Activate the kill switch, stopping all trading immediately.
    pub fn activate_kill_switch(&self) {
        warn!("KILL SWITCH ACTIVATED - all trading halted");
        self.is_killed.store(true, Ordering::SeqCst);
    }

    /// Deactivate the kill switch, allowing trading to resume.
    pub fn deactivate_kill_switch(&self) {
        info!("kill switch deactivated");
        self.is_killed.store(false, Ordering::SeqCst);
    }

    /// Check if trading is currently allowed.
    pub fn is_trading_allowed(&self) -> bool {
        if self.is_killed.load(Ordering::Relaxed) {
            return false;
        }

        let state = self.state.read();
        let now_ms = chrono::Utc::now().timestamp_millis();

        if let Some(resumes_at_ms) = state.circuit_breaker_until_ms {
            if now_ms < resumes_at_ms {
                return false;
            }
        }

        true
    }

    /// Get current risk status.
    pub fn status(&self) -> RiskStatus {
        let state = self.state.read();
        let now_ms = chrono::Utc::now().timestamp_millis();

        let circuit_breaker_active = state
            .circuit_breaker_until_ms
            .map(|t| now_ms < t)
            .unwrap_or(false);

        let drawdown_pct = if state.peak_equity > Decimal::ZERO {
            let drawdown = state.peak_equity - state.current_equity;
            (drawdown / state.peak_equity) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        let price_fn = |_: &str| Some(Decimal::ZERO); // Can't calculate without prices
        let total_exposure = self.positions.total_exposure(price_fn);

        RiskStatus {
            is_killed: self.is_killed.load(Ordering::Relaxed),
            circuit_breaker_active,
            circuit_breaker_until_ms: state.circuit_breaker_until_ms,
            daily_pnl: state.daily_pnl,
            current_drawdown_pct: drawdown_pct,
            peak_equity: state.peak_equity,
            total_exposure,
            open_order_count: self.pending_orders.len() as u32,
        }
    }

    /// Reset daily PnL tracking (call at start of trading day).
    pub fn reset_daily_pnl(&self) {
        let mut state = self.state.write();
        state.daily_pnl = Decimal::ZERO;
        state.peak_equity = Decimal::ZERO;
        state.current_equity = Decimal::ZERO;
        state.trading_start_ms = chrono::Utc::now().timestamp_millis();
        info!("daily PnL reset");
    }

    /// Trigger the circuit breaker.
    fn trigger_circuit_breaker(&self) {
        let mut state = self.state.write();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let resumes_at_ms = now_ms + self.config.circuit_breaker_cooldown_ms;
        state.circuit_breaker_until_ms = Some(resumes_at_ms);

        warn!(
            daily_pnl = %state.daily_pnl,
            cooldown_ms = %self.config.circuit_breaker_cooldown_ms,
            resumes_at_ms = %resumes_at_ms,
            "CIRCUIT BREAKER TRIGGERED"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use execution_core::{
        create_pending_order_registry, create_position_tracker, OrderSide, OrderStatus, OrderType,
        TimeInForce,
    };
    use rust_decimal_macros::dec;
    use strategy_core::OrderIntent;

    fn make_risk_manager() -> RiskManager {
        let config = RiskConfig::conservative();
        let positions = create_position_tracker();
        let pending_orders = create_pending_order_registry();
        RiskManager::new(config, positions, pending_orders)
    }

    fn make_buy_intent(symbol: &str, qty: Decimal, price: Decimal) -> OrderIntent {
        OrderIntent::limit_buy(symbol, qty, price)
    }

    fn make_execution_report(
        symbol: &str,
        side: OrderSide,
        qty: Decimal,
        price: Decimal,
    ) -> ExecutionReport {
        ExecutionReport {
            event_time_ms: chrono::Utc::now().timestamp_millis(),
            symbol: symbol.to_string(),
            client_order_id: "test_order".to_string(),
            side,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::GTC,
            quantity: qty,
            price,
            order_status: OrderStatus::Filled,
            order_id: 12345,
            last_executed_qty: qty,
            cumulative_filled_qty: qty,
            last_executed_price: price,
            commission: dec!(0.01),
            commission_asset: "USDT".to_string(),
            trade_time_ms: chrono::Utc::now().timestamp_millis(),
            trade_id: 1,
            is_maker: true,
        }
    }

    #[test]
    fn test_order_approved_within_limits() {
        let rm = make_risk_manager();
        let intent = make_buy_intent("BTCUSDT", dec!(0.001), dec!(50000));

        // Order notional = 0.001 * 50000 = $50, well within limits
        let result = rm.check_order(&intent, dec!(50000));
        assert!(matches!(result, RiskCheckResult::Approved));
    }

    #[test]
    fn test_kill_switch_rejects_orders() {
        let rm = make_risk_manager();
        rm.activate_kill_switch();

        let intent = make_buy_intent("BTCUSDT", dec!(0.001), dec!(50000));
        let result = rm.check_order(&intent, dec!(50000));

        assert!(matches!(
            result,
            RiskCheckResult::Rejected(RiskRejection::KillSwitchActive)
        ));

        // Deactivate and try again
        rm.deactivate_kill_switch();
        let result = rm.check_order(&intent, dec!(50000));
        assert!(matches!(result, RiskCheckResult::Approved));
    }

    #[test]
    fn test_order_too_large_rejected() {
        let rm = make_risk_manager();

        // Conservative config has $100 max per order
        // 0.01 * 50000 = $500, exceeds limit
        let intent = make_buy_intent("BTCUSDT", dec!(0.01), dec!(50000));
        let result = rm.check_order(&intent, dec!(50000));

        assert!(matches!(
            result,
            RiskCheckResult::Rejected(RiskRejection::OrderTooLarge { .. })
        ));
    }

    #[test]
    fn test_position_limit_exceeded() {
        let config = RiskConfig::conservative(); // $500 max position
        let positions = create_position_tracker();
        let pending_orders = create_pending_order_registry();

        // Simulate having a position already
        let report = make_execution_report("BTCUSDT", OrderSide::Buy, dec!(0.01), dec!(50000));
        positions.update_from_fill(&report);

        let rm = RiskManager::new(config, positions, pending_orders);

        // Current position: 0.01 BTC @ $50k = $500
        // New order: 0.001 BTC @ $50k = $50
        // Total would be: $550, exceeds $500 limit
        let intent = make_buy_intent("BTCUSDT", dec!(0.001), dec!(50000));
        let result = rm.check_order(&intent, dec!(50000));

        assert!(matches!(
            result,
            RiskCheckResult::Rejected(RiskRejection::PositionLimitExceeded { .. })
        ));
    }

    #[test]
    fn test_total_exposure_limit() {
        let config = RiskConfig::conservative(); // $1k total exposure
        let positions = create_position_tracker();
        let pending_orders = create_pending_order_registry();

        // Simulate having positions - note: the risk manager uses a simplified
        // price function that applies the current order's price to all positions.
        // So we set up positions with quantities that will exceed limits when
        // valued at the new order's price.
        //
        // With price = $100 and $1000 limit:
        // - Position 1: 5 units (valued at 5 * 100 = $500)
        // - Position 2: 4 units (valued at 4 * 100 = $400)
        // - Total exposure at check time: $900
        // - New order: 2 units = $200
        // - Would be: $1100, exceeds $1000 limit
        let report1 = make_execution_report("BTCUSDT", OrderSide::Buy, dec!(5), dec!(100));
        positions.update_from_fill(&report1);

        let report2 = make_execution_report("ETHUSDT", OrderSide::Buy, dec!(4), dec!(100));
        positions.update_from_fill(&report2);

        let rm = RiskManager::new(config, positions, pending_orders);

        // New order: 2 units @ $100 = $200
        // Total would be: $900 + $200 = $1100, exceeds $1000 limit
        let intent = make_buy_intent("SOLUSDT", dec!(2), dec!(100));
        let result = rm.check_order(&intent, dec!(100));

        assert!(matches!(
            result,
            RiskCheckResult::Rejected(RiskRejection::TotalExposureLimitExceeded { .. })
        ));
    }

    #[test]
    fn test_circuit_breaker() {
        let rm = make_risk_manager();
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Manually trigger circuit breaker
        rm.trigger_circuit_breaker();

        let intent = make_buy_intent("BTCUSDT", dec!(0.001), dec!(50000));
        let result = rm.check_order(&intent, dec!(50000));

        assert!(matches!(
            result,
            RiskCheckResult::Rejected(RiskRejection::CircuitBreakerActive { resumes_at_ms })
            if resumes_at_ms > now_ms
        ));

        // Check status
        let status = rm.status();
        assert!(status.circuit_breaker_active);
        assert!(!rm.is_trading_allowed());
    }

    #[test]
    fn test_daily_pnl_tracking() {
        let rm = make_risk_manager();

        // Simulate a losing trade
        let report = make_execution_report("BTCUSDT", OrderSide::Buy, dec!(0.01), dec!(50000));
        rm.positions.update_from_fill(&report);
        rm.on_fill(&report);

        let status = rm.status();
        // Initially no realized PnL until position is closed
        assert_eq!(status.daily_pnl, Decimal::ZERO);
    }

    #[test]
    fn test_reset_daily_pnl() {
        let rm = make_risk_manager();

        // Set some state
        {
            let mut state = rm.state.write();
            state.daily_pnl = dec!(-50);
            state.peak_equity = dec!(1000);
            state.current_equity = dec!(950);
        }

        rm.reset_daily_pnl();

        let status = rm.status();
        assert_eq!(status.daily_pnl, Decimal::ZERO);
        assert_eq!(status.peak_equity, Decimal::ZERO);
    }

    #[test]
    fn test_is_trading_allowed() {
        let rm = make_risk_manager();

        // Initially allowed
        assert!(rm.is_trading_allowed());

        // Kill switch stops trading
        rm.activate_kill_switch();
        assert!(!rm.is_trading_allowed());

        // Re-enable
        rm.deactivate_kill_switch();
        assert!(rm.is_trading_allowed());

        // Circuit breaker stops trading
        rm.trigger_circuit_breaker();
        assert!(!rm.is_trading_allowed());
    }

    #[test]
    fn test_too_many_open_orders() {
        let config = RiskConfig {
            max_open_orders_total: 2,
            ..RiskConfig::conservative()
        };
        let positions = create_position_tracker();
        let pending_orders = create_pending_order_registry();

        // Register some pending orders
        pending_orders.register("order1", chrono::Utc::now().timestamp_millis());
        pending_orders.register("order2", chrono::Utc::now().timestamp_millis());

        let rm = RiskManager::new(config, positions, pending_orders);

        let intent = make_buy_intent("BTCUSDT", dec!(0.001), dec!(50000));
        let result = rm.check_order(&intent, dec!(50000));

        assert!(matches!(
            result,
            RiskCheckResult::Rejected(RiskRejection::TooManyOpenOrdersTotal {
                current: 2,
                limit: 2
            })
        ));
    }

    #[test]
    fn test_status() {
        let rm = make_risk_manager();

        let status = rm.status();
        assert!(!status.is_killed);
        assert!(!status.circuit_breaker_active);
        assert_eq!(status.daily_pnl, Decimal::ZERO);
        assert_eq!(status.current_drawdown_pct, Decimal::ZERO);
    }
}
