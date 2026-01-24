//! Risk management configuration.
//!
//! Defines limits and thresholds for protecting against catastrophic losses.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Configuration for risk management.
///
/// These settings control position limits, loss limits, and circuit breakers
/// to prevent catastrophic losses during trading.
#[derive(Debug, Clone)]
pub struct RiskConfig {
    // === Per-Order Limits ===
    /// Maximum notional value for a single order.
    pub max_order_notional: Decimal,

    /// Minimum notional value for a single order.
    pub min_order_notional: Decimal,

    // === Position Limits ===
    /// Maximum position notional per symbol (in quote currency, e.g., USDT).
    pub max_position_notional: Decimal,

    /// Maximum total exposure across all symbols (in quote currency).
    pub max_total_exposure: Decimal,

    // === Loss Limits ===
    /// Maximum daily loss before trading stops (in quote currency).
    /// When realized PnL drops below negative of this value, circuit breaker triggers.
    pub max_daily_loss: Decimal,

    /// Maximum drawdown percentage before trading stops.
    /// Calculated as (peak_equity - current_equity) / peak_equity * 100.
    pub max_drawdown_pct: Decimal,

    // === Order Limits ===
    /// Maximum number of open orders per symbol.
    pub max_open_orders_per_symbol: u32,

    /// Maximum number of open orders across all symbols.
    pub max_open_orders_total: u32,

    // === Circuit Breaker ===
    /// Cooldown period after circuit breaker triggers (in milliseconds).
    pub circuit_breaker_cooldown_ms: i64,

    // === Kill Switch ===
    /// Whether the kill switch is enabled (stops all trading immediately).
    pub enable_kill_switch: bool,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            // Per-order limits
            max_order_notional: dec!(10000), // $10k max per order
            min_order_notional: dec!(10),    // $10 min per order

            // Position limits
            max_position_notional: dec!(50000), // $50k max per symbol
            max_total_exposure: dec!(100000),   // $100k total exposure

            // Loss limits
            max_daily_loss: dec!(1000), // Stop at $1k daily loss
            max_drawdown_pct: dec!(5),  // Stop at 5% drawdown

            // Order limits
            max_open_orders_per_symbol: 5,
            max_open_orders_total: 20,

            // Circuit breaker
            circuit_breaker_cooldown_ms: 300_000, // 5 minute cooldown

            // Kill switch
            enable_kill_switch: false,
        }
    }
}

impl RiskConfig {
    /// Create a new risk config with all default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a conservative config suitable for testnet or initial live testing.
    ///
    /// Uses much smaller limits to minimize potential losses during testing.
    pub fn conservative() -> Self {
        Self {
            max_order_notional: dec!(100),    // $100 max per order
            min_order_notional: dec!(10),     // $10 min per order
            max_position_notional: dec!(500), // $500 max per symbol
            max_total_exposure: dec!(1000),   // $1k total exposure
            max_daily_loss: dec!(50),         // Stop at $50 daily loss
            max_drawdown_pct: dec!(2),        // Stop at 2% drawdown
            max_open_orders_per_symbol: 2,
            max_open_orders_total: 5,
            circuit_breaker_cooldown_ms: 600_000, // 10 minute cooldown
            enable_kill_switch: false,
        }
    }

    /// Builder method to set max daily loss.
    pub fn with_max_daily_loss(mut self, limit: Decimal) -> Self {
        self.max_daily_loss = limit;
        self
    }

    /// Builder method to set max position notional.
    pub fn with_max_position_notional(mut self, limit: Decimal) -> Self {
        self.max_position_notional = limit;
        self
    }

    /// Builder method to set max total exposure.
    pub fn with_max_total_exposure(mut self, limit: Decimal) -> Self {
        self.max_total_exposure = limit;
        self
    }

    /// Builder method to set max order notional.
    pub fn with_max_order_notional(mut self, limit: Decimal) -> Self {
        self.max_order_notional = limit;
        self
    }

    /// Builder method to set circuit breaker cooldown.
    pub fn with_circuit_breaker_cooldown_ms(mut self, cooldown_ms: i64) -> Self {
        self.circuit_breaker_cooldown_ms = cooldown_ms;
        self
    }

    /// Builder method to enable/disable kill switch.
    pub fn with_kill_switch(mut self, enabled: bool) -> Self {
        self.enable_kill_switch = enabled;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RiskConfig::default();

        assert_eq!(config.max_order_notional, dec!(10000));
        assert_eq!(config.min_order_notional, dec!(10));
        assert_eq!(config.max_position_notional, dec!(50000));
        assert_eq!(config.max_total_exposure, dec!(100000));
        assert_eq!(config.max_daily_loss, dec!(1000));
        assert_eq!(config.max_drawdown_pct, dec!(5));
        assert_eq!(config.max_open_orders_per_symbol, 5);
        assert_eq!(config.max_open_orders_total, 20);
        assert_eq!(config.circuit_breaker_cooldown_ms, 300_000);
        assert!(!config.enable_kill_switch);
    }

    #[test]
    fn test_conservative_config() {
        let config = RiskConfig::conservative();

        assert_eq!(config.max_order_notional, dec!(100));
        assert_eq!(config.max_position_notional, dec!(500));
        assert_eq!(config.max_total_exposure, dec!(1000));
        assert_eq!(config.max_daily_loss, dec!(50));
        assert_eq!(config.max_drawdown_pct, dec!(2));
    }

    #[test]
    fn test_builder_methods() {
        let config = RiskConfig::new()
            .with_max_daily_loss(dec!(500))
            .with_max_position_notional(dec!(10000))
            .with_max_total_exposure(dec!(25000))
            .with_circuit_breaker_cooldown_ms(120_000)
            .with_kill_switch(true);

        assert_eq!(config.max_daily_loss, dec!(500));
        assert_eq!(config.max_position_notional, dec!(10000));
        assert_eq!(config.max_total_exposure, dec!(25000));
        assert_eq!(config.circuit_breaker_cooldown_ms, 120_000);
        assert!(config.enable_kill_switch);
    }
}
