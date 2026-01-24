//! Price threshold strategy example.
//!
//! This strategy buys when the price drops below a threshold
//! and sells when it rises above another threshold.

use async_trait::async_trait;
use rust_decimal::Decimal;
use tracing::{debug, info};

use model::MarketEvent;
use strategy_core::{OrderIntent, Signal, Strategy, StrategyContext, StrategyError};

/// Configuration for the price threshold strategy.
#[derive(Debug, Clone)]
pub struct PriceThresholdConfig {
    /// Trading pair symbol (e.g., "BTCUSDT").
    pub symbol: String,
    /// Buy when price drops below this threshold.
    pub buy_threshold: Decimal,
    /// Sell when price rises above this threshold.
    pub sell_threshold: Decimal,
    /// Quantity to trade.
    pub quantity: Decimal,
    /// Minimum time between signals in milliseconds.
    pub cooldown_ms: i64,
}

/// A simple price threshold strategy.
///
/// This strategy demonstrates the basic Strategy trait implementation.
/// It monitors price and generates buy signals when price drops below
/// a threshold, and sell signals when price rises above another threshold.
pub struct PriceThresholdStrategy {
    id: String,
    config: PriceThresholdConfig,
    symbols: Vec<String>,
    has_position: bool,
    last_signal_ms: i64,
}

impl PriceThresholdStrategy {
    /// Create a new price threshold strategy.
    pub fn new(id: impl Into<String>, config: PriceThresholdConfig) -> Self {
        let symbols = vec![config.symbol.clone()];
        Self {
            id: id.into(),
            config,
            symbols,
            has_position: false,
            last_signal_ms: 0,
        }
    }

    /// Check if cooldown has elapsed since last signal.
    fn cooldown_elapsed(&self, current_time_ms: i64) -> bool {
        // Allow first signal (when last_signal_ms is 0)
        if self.last_signal_ms == 0 {
            return true;
        }
        current_time_ms - self.last_signal_ms >= self.config.cooldown_ms
    }
}

#[async_trait]
impl Strategy for PriceThresholdStrategy {
    fn id(&self) -> &str {
        &self.id
    }

    fn symbols(&self) -> Option<&[String]> {
        Some(&self.symbols)
    }

    async fn on_start(&mut self, _ctx: &StrategyContext) -> Result<(), StrategyError> {
        info!(
            strategy_id = %self.id,
            symbol = %self.config.symbol,
            buy_threshold = %self.config.buy_threshold,
            sell_threshold = %self.config.sell_threshold,
            quantity = %self.config.quantity,
            "Price threshold strategy started"
        );
        Ok(())
    }

    async fn on_market_event(
        &mut self,
        event: &MarketEvent,
        ctx: &StrategyContext,
    ) -> Result<Option<Signal>, StrategyError> {
        let MarketEvent::Trade(trade) = event;

        // Only process trades for our symbol
        if trade.symbol != self.config.symbol {
            return Ok(None);
        }

        let price = trade.price;

        // Check cooldown
        if !self.cooldown_elapsed(ctx.timestamp_ms) {
            return Ok(None);
        }

        // Buy signal: price below threshold and no position
        if price < self.config.buy_threshold && !self.has_position {
            debug!(
                strategy_id = %self.id,
                price = %price,
                threshold = %self.config.buy_threshold,
                "Price below buy threshold"
            );

            self.has_position = true;
            self.last_signal_ms = ctx.timestamp_ms;

            return Ok(Some(Signal::with_reason(
                &self.id,
                strategy_core::SignalKind::Order(OrderIntent::market_buy(
                    &self.config.symbol,
                    self.config.quantity,
                )),
                format!(
                    "Price {} below buy threshold {}",
                    price, self.config.buy_threshold
                ),
            )));
        }

        // Sell signal: price above threshold and has position
        if price > self.config.sell_threshold && self.has_position {
            debug!(
                strategy_id = %self.id,
                price = %price,
                threshold = %self.config.sell_threshold,
                "Price above sell threshold"
            );

            self.has_position = false;
            self.last_signal_ms = ctx.timestamp_ms;

            return Ok(Some(Signal::with_reason(
                &self.id,
                strategy_core::SignalKind::Order(OrderIntent::market_sell(
                    &self.config.symbol,
                    self.config.quantity,
                )),
                format!(
                    "Price {} above sell threshold {}",
                    price, self.config.sell_threshold
                ),
            )));
        }

        Ok(None)
    }

    async fn on_stop(&mut self, _ctx: &StrategyContext) -> Result<(), StrategyError> {
        info!(
            strategy_id = %self.id,
            has_position = self.has_position,
            "Price threshold strategy stopped"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Exchange, MakerSide, Trade};
    use rust_decimal_macros::dec;
    use strategy_core::create_market_state;

    fn make_trade(price: Decimal) -> MarketEvent {
        MarketEvent::Trade(Trade {
            exchange: Exchange::Binance,
            symbol: "BTCUSDT".to_string(),
            price,
            qty: dec!(0.1),
            trade_id: 1,
            timestamp_ms: 1000,
            maker_side: MakerSide::Buyer,
        })
    }

    fn make_context(timestamp_ms: i64) -> StrategyContext {
        StrategyContext::new(timestamp_ms, create_market_state())
    }

    #[tokio::test]
    async fn test_buy_signal_below_threshold() {
        let config = PriceThresholdConfig {
            symbol: "BTCUSDT".to_string(),
            buy_threshold: dec!(50000),
            sell_threshold: dec!(55000),
            quantity: dec!(0.001),
            cooldown_ms: 1000,
        };

        let mut strategy = PriceThresholdStrategy::new("test", config);

        // Price below buy threshold should generate buy signal
        let event = make_trade(dec!(49000));
        let ctx = make_context(1000);

        let signal = strategy.on_market_event(&event, &ctx).await.unwrap();
        assert!(signal.is_some());

        let signal = signal.unwrap();
        assert!(matches!(signal.kind, strategy_core::SignalKind::Order(_)));
    }

    #[tokio::test]
    async fn test_no_buy_when_has_position() {
        let config = PriceThresholdConfig {
            symbol: "BTCUSDT".to_string(),
            buy_threshold: dec!(50000),
            sell_threshold: dec!(55000),
            quantity: dec!(0.001),
            cooldown_ms: 1000,
        };

        let mut strategy = PriceThresholdStrategy::new("test", config);
        strategy.has_position = true; // Already have position

        // Price below threshold but already have position
        let event = make_trade(dec!(49000));
        let ctx = make_context(1000);

        let signal = strategy.on_market_event(&event, &ctx).await.unwrap();
        assert!(signal.is_none());
    }

    #[tokio::test]
    async fn test_sell_signal_above_threshold() {
        let config = PriceThresholdConfig {
            symbol: "BTCUSDT".to_string(),
            buy_threshold: dec!(50000),
            sell_threshold: dec!(55000),
            quantity: dec!(0.001),
            cooldown_ms: 1000,
        };

        let mut strategy = PriceThresholdStrategy::new("test", config);
        strategy.has_position = true; // Have position to sell

        // Price above sell threshold should generate sell signal
        let event = make_trade(dec!(56000));
        let ctx = make_context(1000);

        let signal = strategy.on_market_event(&event, &ctx).await.unwrap();
        assert!(signal.is_some());
    }

    #[tokio::test]
    async fn test_cooldown_prevents_signal() {
        let config = PriceThresholdConfig {
            symbol: "BTCUSDT".to_string(),
            buy_threshold: dec!(50000),
            sell_threshold: dec!(55000),
            quantity: dec!(0.001),
            cooldown_ms: 5000,
        };

        let mut strategy = PriceThresholdStrategy::new("test", config);

        // First signal should work (buy at 49000)
        let buy_event = make_trade(dec!(49000));
        let ctx = make_context(1000);
        let signal = strategy.on_market_event(&buy_event, &ctx).await.unwrap();
        assert!(signal.is_some(), "First buy signal should work");

        // Immediately try again - should be blocked by cooldown
        // (strategy now has_position=true, so try sell at 56000)
        let sell_event = make_trade(dec!(56000));
        let ctx = make_context(2000); // Only 1 second later
        let signal = strategy.on_market_event(&sell_event, &ctx).await.unwrap();
        assert!(signal.is_none(), "Should be blocked by cooldown");

        // After cooldown should work (6 seconds after first signal at t=1000)
        let ctx = make_context(6001); // 5001ms after first signal
        let signal = strategy.on_market_event(&sell_event, &ctx).await.unwrap();
        assert!(signal.is_some(), "Sell signal should work after cooldown");
    }

    #[tokio::test]
    async fn test_no_signal_in_neutral_zone() {
        let config = PriceThresholdConfig {
            symbol: "BTCUSDT".to_string(),
            buy_threshold: dec!(50000),
            sell_threshold: dec!(55000),
            quantity: dec!(0.001),
            cooldown_ms: 1000,
        };

        let mut strategy = PriceThresholdStrategy::new("test", config);

        // Price between thresholds should not generate signal
        let event = make_trade(dec!(52000));
        let ctx = make_context(1000);

        let signal = strategy.on_market_event(&event, &ctx).await.unwrap();
        assert!(signal.is_none());
    }
}
