//! Signal processor with validation and rate limiting.

use std::collections::HashMap;
use std::time::Instant;

use rust_decimal::Decimal;
use tracing::{debug, warn};

use execution_core::{generate_client_order_id, OrderType};
use strategy_core::{OrderIntent, Signal, SignalKind};

use crate::error::SignalError;

/// Configuration for the signal processor.
#[derive(Debug, Clone)]
pub struct SignalProcessorConfig {
    /// Maximum orders per second per strategy.
    pub max_orders_per_second: u32,
    /// Minimum order notional value.
    pub min_order_notional: Decimal,
    /// Maximum order notional value.
    pub max_order_notional: Decimal,
    /// Prefix for generated client order IDs.
    pub order_id_prefix: String,
    /// Whether live trading is enabled.
    pub live_trading: bool,
}

impl Default for SignalProcessorConfig {
    fn default() -> Self {
        Self {
            max_orders_per_second: 5,
            min_order_notional: Decimal::from(10), // $10 minimum
            max_order_notional: Decimal::from(100_000), // $100k maximum
            order_id_prefix: "bot".to_string(),
            live_trading: false, // Dry run by default
        }
    }
}

/// A processed signal ready for execution.
#[derive(Debug)]
pub struct ProcessedSignal {
    /// The original signal.
    pub signal: Signal,
    /// Generated client order ID (for order signals).
    pub client_order_id: Option<String>,
}

/// Signal processor that validates signals and enforces rate limits.
pub struct SignalProcessor {
    config: SignalProcessorConfig,
    /// Rate limit tracking: strategy_id -> (window_start, order_count)
    rate_limits: HashMap<String, (Instant, u32)>,
}

impl SignalProcessor {
    /// Create a new signal processor with the given configuration.
    pub fn new(config: SignalProcessorConfig) -> Self {
        Self {
            config,
            rate_limits: HashMap::new(),
        }
    }

    /// Process a signal, performing validation and rate limiting.
    ///
    /// Returns a `ProcessedSignal` if the signal passes all checks.
    pub fn process(&mut self, signal: Signal) -> Result<ProcessedSignal, SignalError> {
        // Check rate limits
        self.check_rate_limit(&signal.strategy_id)?;

        // Validate based on signal kind
        match &signal.kind {
            SignalKind::Order(intent) => {
                self.validate_order(intent)?;
            }
            SignalKind::Cancel(_) => {
                // Cancel requests don't need additional validation
            }
            SignalKind::Modify(intent) => {
                // Validate new price/quantity if provided
                if intent.new_price.is_none() && intent.new_quantity.is_none() {
                    return Err(SignalError::InvalidOrder(
                        "modify intent must change price or quantity".to_string(),
                    ));
                }
            }
        }

        // Generate client order ID for order signals
        let client_order_id = if matches!(signal.kind, SignalKind::Order(_)) {
            Some(generate_client_order_id(&self.config.order_id_prefix))
        } else {
            None
        };

        // Update rate limit counter
        self.increment_rate_limit(&signal.strategy_id);

        debug!(
            strategy_id = %signal.strategy_id,
            signal_kind = ?std::mem::discriminant(&signal.kind),
            client_order_id = ?client_order_id,
            "signal processed"
        );

        Ok(ProcessedSignal {
            signal,
            client_order_id,
        })
    }

    /// Check if the strategy has exceeded its rate limit.
    fn check_rate_limit(&self, strategy_id: &str) -> Result<(), SignalError> {
        if let Some((window_start, count)) = self.rate_limits.get(strategy_id) {
            let elapsed = window_start.elapsed();
            if elapsed.as_secs() < 1 && *count >= self.config.max_orders_per_second {
                warn!(
                    strategy_id = %strategy_id,
                    count = %count,
                    max = %self.config.max_orders_per_second,
                    "rate limit exceeded"
                );
                return Err(SignalError::RateLimitExceeded(
                    self.config.max_orders_per_second,
                ));
            }
        }
        Ok(())
    }

    /// Increment the rate limit counter for a strategy.
    fn increment_rate_limit(&mut self, strategy_id: &str) {
        let now = Instant::now();
        let entry = self
            .rate_limits
            .entry(strategy_id.to_string())
            .or_insert((now, 0));

        // Reset window if more than 1 second has passed
        if entry.0.elapsed().as_secs() >= 1 {
            *entry = (now, 1);
        } else {
            entry.1 += 1;
        }
    }

    /// Validate an order intent.
    fn validate_order(&self, intent: &OrderIntent) -> Result<(), SignalError> {
        // Check for valid quantity
        if intent.quantity <= Decimal::ZERO {
            return Err(SignalError::InvalidOrder(
                "quantity must be positive".to_string(),
            ));
        }

        // Check for price on limit orders
        if matches!(intent.order_type, OrderType::Limit) && intent.price.is_none() {
            return Err(SignalError::MissingPrice);
        }

        // Validate price if present
        if let Some(price) = intent.price {
            if price <= Decimal::ZERO {
                return Err(SignalError::InvalidOrder(
                    "price must be positive".to_string(),
                ));
            }

            // Check notional value for limit orders
            let notional = price * intent.quantity;
            self.validate_notional(notional)?;
        }

        Ok(())
    }

    /// Validate order notional value.
    fn validate_notional(&self, notional: Decimal) -> Result<(), SignalError> {
        if notional < self.config.min_order_notional {
            return Err(SignalError::NotionalTooSmall(
                notional.to_string(),
                self.config.min_order_notional.to_string(),
            ));
        }

        if notional > self.config.max_order_notional {
            return Err(SignalError::NotionalTooLarge(
                notional.to_string(),
                self.config.max_order_notional.to_string(),
            ));
        }

        Ok(())
    }

    /// Returns whether live trading is enabled.
    pub fn is_live(&self) -> bool {
        self.config.live_trading
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use strategy_core::OrderIntent;

    fn make_processor() -> SignalProcessor {
        SignalProcessor::new(SignalProcessorConfig {
            max_orders_per_second: 2,
            min_order_notional: dec!(10),
            max_order_notional: dec!(1000),
            order_id_prefix: "test".to_string(),
            live_trading: false,
        })
    }

    #[test]
    fn test_process_valid_order() {
        let mut processor = make_processor();

        let signal = Signal::order(
            "test_strategy",
            OrderIntent::limit_buy("BTCUSDT", dec!(0.001), dec!(50000)),
        );

        let result = processor.process(signal);
        assert!(result.is_ok());

        let processed = result.unwrap();
        assert!(processed.client_order_id.is_some());
        assert!(processed.client_order_id.unwrap().starts_with("test_"));
    }

    #[test]
    fn test_rate_limit_exceeded() {
        let mut processor = make_processor();

        // First two orders should succeed
        for _ in 0..2 {
            let signal = Signal::order(
                "test_strategy",
                OrderIntent::limit_buy("BTCUSDT", dec!(0.001), dec!(50000)),
            );
            assert!(processor.process(signal).is_ok());
        }

        // Third order should be rate limited
        let signal = Signal::order(
            "test_strategy",
            OrderIntent::limit_buy("BTCUSDT", dec!(0.001), dec!(50000)),
        );
        let result = processor.process(signal);
        assert!(matches!(result, Err(SignalError::RateLimitExceeded(_))));
    }

    #[test]
    fn test_notional_too_small() {
        let mut processor = make_processor();

        // $5 notional (0.0001 * 50000) is below $10 minimum
        let signal = Signal::order(
            "test_strategy",
            OrderIntent::limit_buy("BTCUSDT", dec!(0.0001), dec!(50000)),
        );

        let result = processor.process(signal);
        assert!(matches!(result, Err(SignalError::NotionalTooSmall(_, _))));
    }

    #[test]
    fn test_notional_too_large() {
        let mut processor = make_processor();

        // $5000 notional (0.1 * 50000) is above $1000 maximum
        let signal = Signal::order(
            "test_strategy",
            OrderIntent::limit_buy("BTCUSDT", dec!(0.1), dec!(50000)),
        );

        let result = processor.process(signal);
        assert!(matches!(result, Err(SignalError::NotionalTooLarge(_, _))));
    }

    #[test]
    fn test_missing_price_for_limit() {
        let mut processor = make_processor();

        let signal = Signal::order(
            "test_strategy",
            OrderIntent {
                symbol: "BTCUSDT".to_string(),
                side: strategy_core::OrderSide::Buy,
                order_type: OrderType::Limit,
                quantity: dec!(0.001),
                price: None, // Missing price!
                time_in_force: None,
            },
        );

        let result = processor.process(signal);
        assert!(matches!(result, Err(SignalError::MissingPrice)));
    }

    #[test]
    fn test_invalid_quantity() {
        let mut processor = make_processor();

        let signal = Signal::order(
            "test_strategy",
            OrderIntent {
                symbol: "BTCUSDT".to_string(),
                side: strategy_core::OrderSide::Buy,
                order_type: OrderType::Market,
                quantity: dec!(0), // Zero quantity!
                price: None,
                time_in_force: None,
            },
        );

        let result = processor.process(signal);
        assert!(matches!(result, Err(SignalError::InvalidOrder(_))));
    }

    #[test]
    fn test_market_order_no_notional_check() {
        let mut processor = make_processor();

        // Market orders don't have a price, so notional can't be checked
        let signal = Signal::order(
            "test_strategy",
            OrderIntent::market_buy("BTCUSDT", dec!(0.0001)),
        );

        // Should succeed (no price to calculate notional)
        let result = processor.process(signal);
        assert!(result.is_ok());
    }
}
