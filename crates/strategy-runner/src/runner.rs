//! Strategy runner - main execution loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use rust_decimal::Decimal;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, trace, warn};

use binance_rest::BinanceRestClient;
use connector_core::EventReceiver;
use execution_core::{ExecutionReport, SharedPendingOrderRegistry, SharedPositionTracker};
use model::MarketEvent;
use strategy_core::{
    create_market_state, BoxedStrategy, SharedMarketState, Signal, SignalKind, StrategyContext,
};

use crate::error::RunnerError;
use crate::risk_manager::{RiskAction, RiskCheckResult, RiskManager};
use crate::signal_processor::{SignalProcessor, SignalProcessorConfig};
use crate::timer::TimerManager;

/// Configuration for the strategy runner.
#[derive(Debug, Clone)]
pub struct StrategyRunnerConfig {
    /// Signal processor configuration.
    pub signal_processor: SignalProcessorConfig,
    /// Timeout for stale pending orders (in milliseconds).
    /// Orders in the pending registry older than this will be cleaned up.
    /// Default: 5 minutes (300,000 ms).
    pub stale_order_timeout_ms: i64,
}

impl Default for StrategyRunnerConfig {
    fn default() -> Self {
        Self {
            signal_processor: SignalProcessorConfig::default(),
            stale_order_timeout_ms: 300_000, // 5 minutes
        }
    }
}

/// The strategy runner orchestrates strategy execution.
///
/// It receives market events and execution reports, dispatches them to
/// registered strategies, and processes generated signals.
pub struct StrategyRunner {
    /// Runner configuration.
    config: StrategyRunnerConfig,
    /// Registered strategies.
    strategies: Vec<BoxedStrategy>,
    /// Signal processor for validation and rate limiting.
    signal_processor: SignalProcessor,
    /// Timer manager for periodic callbacks.
    timer_manager: TimerManager,
    /// Shared market state.
    market_state: SharedMarketState,
    /// REST client for order execution (optional for dry-run).
    rest_client: Option<Arc<BinanceRestClient>>,
    /// Pending order registry for correlation.
    pending_orders: Option<SharedPendingOrderRegistry>,
    /// Position tracker for risk management.
    position_tracker: Option<SharedPositionTracker>,
    /// Risk manager for pre-trade checks (wrapped in Arc for shared access).
    risk_manager: Option<Arc<RiskManager>>,
    /// Current prices by symbol (for risk calculations).
    current_prices: Arc<RwLock<HashMap<String, Decimal>>>,
    /// Last time stale orders were cleaned up.
    last_stale_cleanup_ms: i64,
}

impl StrategyRunner {
    /// Create a new strategy runner with the given configuration.
    pub fn new(config: StrategyRunnerConfig) -> Self {
        Self {
            signal_processor: SignalProcessor::new(config.signal_processor.clone()),
            config,
            strategies: Vec::new(),
            timer_manager: TimerManager::new(),
            market_state: create_market_state(),
            rest_client: None,
            pending_orders: None,
            position_tracker: None,
            risk_manager: None,
            current_prices: Arc::new(RwLock::new(HashMap::new())),
            last_stale_cleanup_ms: 0,
        }
    }

    /// Set the REST client for live trading.
    pub fn with_rest_client(mut self, client: Arc<BinanceRestClient>) -> Self {
        self.rest_client = Some(client);
        self
    }

    /// Set the pending order registry for order correlation.
    pub fn with_pending_orders(mut self, registry: SharedPendingOrderRegistry) -> Self {
        self.pending_orders = Some(registry);
        self
    }

    /// Set the position tracker for tracking fills.
    pub fn with_position_tracker(mut self, tracker: SharedPositionTracker) -> Self {
        self.position_tracker = Some(tracker);
        self
    }

    /// Set the risk manager for pre-trade checks.
    pub fn with_risk_manager(mut self, manager: Arc<RiskManager>) -> Self {
        self.risk_manager = Some(manager);
        self
    }

    /// Register a strategy with the runner.
    pub fn register_strategy(&mut self, strategy: BoxedStrategy) {
        let strategy_id = strategy.id().to_string();

        // Register timer if the strategy has one
        if let Some(interval) = strategy.timer_interval() {
            self.timer_manager.register(&strategy_id, interval);
            info!(
                strategy_id = %strategy_id,
                interval_ms = %interval.as_millis(),
                "registered strategy timer"
            );
        }

        info!(strategy_id = %strategy_id, "registered strategy");
        self.strategies.push(strategy);
    }

    /// Get a reference to the shared market state.
    pub fn market_state(&self) -> &SharedMarketState {
        &self.market_state
    }

    /// Run the strategy loop.
    ///
    /// This method runs until shutdown is signaled.
    pub async fn run(
        mut self,
        mut market_rx: EventReceiver,
        mut execution_rx: mpsc::Receiver<ExecutionReport>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> Result<(), RunnerError> {
        info!(
            strategy_count = self.strategies.len(),
            live_trading = self.signal_processor.is_live(),
            "starting strategy runner"
        );

        // Call on_start for all strategies
        self.start_strategies().await?;

        // Calculate timer interval for select
        let timer_interval = Duration::from_millis(100); // Check timers every 100ms

        loop {
            tokio::select! {
                biased;

                // Shutdown signal (highest priority)
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("shutdown signal received");
                        break;
                    }
                }

                // Timer tick
                _ = tokio::time::sleep(timer_interval), if !self.timer_manager.is_empty() => {
                    self.handle_timers().await?;
                }

                // Execution reports (higher priority than market events)
                Some(report) = execution_rx.recv() => {
                    self.handle_execution_report(report).await?;
                }

                // Market events
                Some(event) = market_rx.recv() => {
                    self.handle_market_event(event).await?;
                }

                // All channels closed
                else => {
                    warn!("all channels closed");
                    break;
                }
            }
        }

        // Call on_stop for all strategies
        self.stop_strategies().await?;

        info!("strategy runner stopped");
        Ok(())
    }

    /// Call on_start for all strategies.
    async fn start_strategies(&mut self) -> Result<(), RunnerError> {
        let ctx = self.make_context();

        for strategy in &mut self.strategies {
            let strategy_id = strategy.id().to_string();
            if let Err(e) = strategy.on_start(&ctx).await {
                error!(strategy_id = %strategy_id, error = %e, "strategy start failed");
                return Err(e.into());
            }
            debug!(strategy_id = %strategy_id, "strategy started");
        }

        Ok(())
    }

    /// Call on_stop for all strategies.
    async fn stop_strategies(&mut self) -> Result<(), RunnerError> {
        let ctx = self.make_context();

        for strategy in &mut self.strategies {
            let strategy_id = strategy.id().to_string();
            if let Err(e) = strategy.on_stop(&ctx).await {
                error!(strategy_id = %strategy_id, error = %e, "strategy stop failed");
                // Continue stopping other strategies
            } else {
                debug!(strategy_id = %strategy_id, "strategy stopped");
            }
        }

        Ok(())
    }

    /// Handle a market event.
    async fn handle_market_event(&mut self, event: MarketEvent) -> Result<(), RunnerError> {
        // Log the trade
        if let MarketEvent::Trade(ref trade) = event {
            trace!(
                id = trade.trade_id,
                symbol = %trade.symbol,
                price = %trade.price,
                qty = %trade.qty,
                side = ?trade.maker_side,
                "trade"
            );
        }

        // Update market state
        self.update_market_state(&event);

        let ctx = self.make_context();

        // Collect signals first to avoid borrow issues
        let mut signals = Vec::new();

        for strategy in &mut self.strategies {
            // Check symbol filter
            if let Some(symbols) = strategy.symbols() {
                let event_symbol = match &event {
                    MarketEvent::Trade(t) => &t.symbol,
                };
                if !symbols.iter().any(|s| s == event_symbol) {
                    continue;
                }
            }

            match strategy.on_market_event(&event, &ctx).await {
                Ok(Some(signal)) => {
                    signals.push(signal);
                }
                Ok(None) => {}
                Err(e) => {
                    error!(
                        strategy_id = %strategy.id(),
                        error = %e,
                        "strategy error on market event"
                    );
                }
            }
        }

        // Now process collected signals
        for signal in signals {
            self.process_signal(signal).await;
        }

        Ok(())
    }

    /// Handle an execution report.
    async fn handle_execution_report(
        &mut self,
        report: ExecutionReport,
    ) -> Result<(), RunnerError> {
        debug!(
            client_order_id = %report.client_order_id,
            status = ?report.order_status,
            "received execution report"
        );

        // Update position tracker with fill information
        if let Some(ref tracker) = self.position_tracker {
            tracker.update_from_fill(&report);
        }

        // Update risk manager with fill information
        if let Some(ref risk_manager) = self.risk_manager {
            risk_manager.on_fill(&report);
        }

        // Clean up pending order registry when order reaches terminal state
        if report.order_status.is_terminal() {
            if let Some(ref pending) = self.pending_orders {
                pending.remove(&report.client_order_id);
                debug!(
                    client_order_id = %report.client_order_id,
                    "removed completed order from pending registry"
                );
            }
        }

        let ctx = self.make_context();

        // Collect signals first to avoid borrow issues
        let mut signals = Vec::new();

        for strategy in &mut self.strategies {
            match strategy.on_order_update(&report, &ctx).await {
                Ok(Some(signal)) => {
                    signals.push(signal);
                }
                Ok(None) => {}
                Err(e) => {
                    error!(
                        strategy_id = %strategy.id(),
                        error = %e,
                        "strategy error on order update"
                    );
                }
            }
        }

        // Now process collected signals
        for signal in signals {
            self.process_signal(signal).await;
        }

        Ok(())
    }

    /// Handle timer callbacks.
    async fn handle_timers(&mut self) -> Result<(), RunnerError> {
        // Periodically clean up stale pending orders (every minute)
        let now = chrono_timestamp_ms();
        if now - self.last_stale_cleanup_ms > 60_000 {
            self.cleanup_stale_orders(now);
            self.last_stale_cleanup_ms = now;
        }

        // Periodic risk checks
        if let Some(ref risk_manager) = self.risk_manager {
            let prices = self.current_prices.read().clone();
            if let Some(action) = risk_manager.periodic_check(&prices, now) {
                self.handle_risk_action(action).await;
            }
        }

        let due_strategies = self.timer_manager.check_due();

        if due_strategies.is_empty() {
            return Ok(());
        }

        let ctx = self.make_context();

        // Collect signals first to avoid borrow issues
        let mut signals = Vec::new();

        for strategy_id in due_strategies {
            // Find the strategy by ID
            for strategy in &mut self.strategies {
                if strategy.id() == strategy_id {
                    match strategy.on_timer(&ctx).await {
                        Ok(Some(signal)) => {
                            signals.push(signal);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            error!(
                                strategy_id = %strategy_id,
                                error = %e,
                                "strategy error on timer"
                            );
                        }
                    }
                    break;
                }
            }
        }

        // Now process collected signals
        for signal in signals {
            self.process_signal(signal).await;
        }

        Ok(())
    }

    /// Process a signal generated by a strategy.
    async fn process_signal(&mut self, mut signal: Signal) {
        // Set timestamp
        signal.generated_at_ms = chrono_timestamp_ms();

        // Process through signal processor (rate limits, basic validation)
        let processed = match self.signal_processor.process(signal) {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "signal rejected by processor");
                return;
            }
        };

        // Risk check for order signals
        if let SignalKind::Order(ref intent) = processed.signal.kind {
            if let Some(ref risk_manager) = self.risk_manager {
                // Get current price for the symbol
                let current_price = self
                    .current_prices
                    .read()
                    .get(&intent.symbol)
                    .copied()
                    .or(intent.price)
                    .unwrap_or(Decimal::ZERO);

                match risk_manager.check_order(intent, current_price) {
                    RiskCheckResult::Approved => {
                        debug!(
                            symbol = %intent.symbol,
                            "order approved by risk manager"
                        );
                    }
                    RiskCheckResult::Rejected(reason) => {
                        warn!(
                            symbol = %intent.symbol,
                            reason = %reason,
                            "order rejected by risk manager"
                        );
                        return;
                    }
                }
            }
        }

        // Execute signal
        if self.signal_processor.is_live() {
            self.execute_signal_live(processed).await;
        } else {
            self.execute_signal_dry(processed);
        }
    }

    /// Handle a risk action from the risk manager.
    async fn handle_risk_action(&self, action: RiskAction) {
        match action {
            RiskAction::TriggerCircuitBreaker => {
                warn!("circuit breaker triggered - trading paused");
                // The circuit breaker state is already set in the risk manager.
                // New orders will be rejected until it clears.
            }
            RiskAction::CancelAllOrders => {
                warn!("risk action: canceling all open orders");
                // TODO: Implement cancel all orders when we have order tracking
            }
            RiskAction::CloseAllPositions => {
                warn!("risk action: closing all positions");
                // TODO: Implement close all positions when we have position management
            }
        }
    }

    /// Execute a signal in dry-run mode (logging only).
    fn execute_signal_dry(&self, processed: crate::signal_processor::ProcessedSignal) {
        match &processed.signal.kind {
            SignalKind::Order(intent) => {
                info!(
                    strategy_id = %processed.signal.strategy_id,
                    client_order_id = ?processed.client_order_id,
                    symbol = %intent.symbol,
                    side = ?intent.side,
                    order_type = ?intent.order_type,
                    quantity = %intent.quantity,
                    price = ?intent.price,
                    reason = ?processed.signal.reason,
                    "[DRY RUN] would place order"
                );
            }
            SignalKind::Cancel(intent) => {
                info!(
                    strategy_id = %processed.signal.strategy_id,
                    symbol = %intent.symbol,
                    client_order_id = %intent.client_order_id,
                    "[DRY RUN] would cancel order"
                );
            }
            SignalKind::Modify(intent) => {
                info!(
                    strategy_id = %processed.signal.strategy_id,
                    symbol = %intent.symbol,
                    original_client_order_id = %intent.original_client_order_id,
                    new_price = ?intent.new_price,
                    new_quantity = ?intent.new_quantity,
                    "[DRY RUN] would modify order"
                );
            }
        }
    }

    /// Execute a signal in live trading mode.
    async fn execute_signal_live(&mut self, processed: crate::signal_processor::ProcessedSignal) {
        let Some(rest_client) = &self.rest_client else {
            error!("live trading enabled but no REST client configured");
            return;
        };

        match &processed.signal.kind {
            SignalKind::Order(intent) => {
                let client_order_id = processed.client_order_id.as_ref().unwrap();

                // Register pending order
                if let Some(ref pending) = self.pending_orders {
                    pending.register(client_order_id, chrono_timestamp_ms());
                }

                info!(
                    strategy_id = %processed.signal.strategy_id,
                    client_order_id = %client_order_id,
                    symbol = %intent.symbol,
                    side = ?intent.side,
                    order_type = ?intent.order_type,
                    quantity = %intent.quantity,
                    price = ?intent.price,
                    stop_price = ?intent.stop_price,
                    "placing order"
                );

                match rest_client
                    .place_order(
                        &intent.symbol,
                        intent.side,
                        intent.order_type,
                        intent.quantity,
                        intent.price,
                        intent.stop_price,
                        intent.time_in_force,
                        client_order_id,
                    )
                    .await
                {
                    Ok(response) => {
                        info!(
                            client_order_id = %client_order_id,
                            order_id = response.order_id,
                            status = ?response.status,
                            "order placed successfully"
                        );
                    }
                    Err(e) => {
                        error!(
                            client_order_id = %client_order_id,
                            error = %e,
                            "failed to place order"
                        );
                    }
                }
            }
            SignalKind::Cancel(intent) => {
                info!(
                    strategy_id = %processed.signal.strategy_id,
                    symbol = %intent.symbol,
                    client_order_id = %intent.client_order_id,
                    "canceling order"
                );

                match rest_client
                    .cancel_order_by_client_id(&intent.symbol, &intent.client_order_id)
                    .await
                {
                    Ok(_) => {
                        info!(
                            client_order_id = %intent.client_order_id,
                            "order canceled successfully"
                        );
                    }
                    Err(e) => {
                        error!(
                            client_order_id = %intent.client_order_id,
                            error = %e,
                            "failed to cancel order"
                        );
                    }
                }
            }
            SignalKind::Modify(_intent) => {
                // Modify is typically implemented as cancel + new order
                // For now, log a warning
                warn!(
                    strategy_id = %processed.signal.strategy_id,
                    "modify orders not yet implemented - use cancel + new order"
                );
            }
        }
    }

    /// Update market state from a market event.
    fn update_market_state(&self, event: &MarketEvent) {
        match event {
            MarketEvent::Trade(trade) => {
                // Update shared market state
                self.market_state
                    .update_last_price(&trade.symbol, trade.price);

                // Update current prices for risk calculations
                self.current_prices
                    .write()
                    .insert(trade.symbol.clone(), trade.price);
            }
        }
    }

    /// Clean up stale pending orders to prevent memory leaks.
    fn cleanup_stale_orders(&self, current_time_ms: i64) {
        if let Some(ref pending) = self.pending_orders {
            let before = pending.len();
            pending.cleanup_stale(self.config.stale_order_timeout_ms, current_time_ms);
            let after = pending.len();
            if before != after {
                info!(
                    removed = before - after,
                    remaining = after,
                    "cleaned up stale pending orders"
                );
            }
        }
    }

    /// Create a strategy context for the current moment.
    fn make_context(&self) -> StrategyContext {
        StrategyContext::new(chrono_timestamp_ms(), Arc::clone(&self.market_state))
    }
}

/// Get current timestamp in milliseconds.
fn chrono_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rust_decimal_macros::dec;
    use std::sync::atomic::{AtomicU32, Ordering};
    use strategy_core::{OrderIntent, Strategy, StrategyError};

    struct CountingStrategy {
        id: String,
        market_event_count: AtomicU32,
        timer_count: AtomicU32,
    }

    impl CountingStrategy {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                market_event_count: AtomicU32::new(0),
                timer_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl Strategy for CountingStrategy {
        fn id(&self) -> &str {
            &self.id
        }

        fn timer_interval(&self) -> Option<Duration> {
            Some(Duration::from_millis(50))
        }

        async fn on_market_event(
            &mut self,
            _event: &MarketEvent,
            _ctx: &StrategyContext,
        ) -> Result<Option<Signal>, StrategyError> {
            self.market_event_count.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn on_timer(
            &mut self,
            _ctx: &StrategyContext,
        ) -> Result<Option<Signal>, StrategyError> {
            self.timer_count.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
    }

    struct SignalStrategy {
        id: String,
        #[allow(dead_code)]
        should_signal: bool,
    }

    #[async_trait]
    impl Strategy for SignalStrategy {
        fn id(&self) -> &str {
            &self.id
        }

        async fn on_market_event(
            &mut self,
            _event: &MarketEvent,
            _ctx: &StrategyContext,
        ) -> Result<Option<Signal>, StrategyError> {
            if self.should_signal {
                Ok(Some(Signal::order(
                    &self.id,
                    OrderIntent::limit_buy("BTCUSDT", dec!(0.001), dec!(50000)),
                )))
            } else {
                Ok(None)
            }
        }
    }

    #[test]
    fn test_runner_creation() {
        let config = StrategyRunnerConfig::default();
        let runner = StrategyRunner::new(config);
        assert!(runner.strategies.is_empty());
    }

    #[test]
    fn test_register_strategy() {
        let config = StrategyRunnerConfig::default();
        let mut runner = StrategyRunner::new(config);

        let strategy = Box::new(CountingStrategy::new("test"));
        runner.register_strategy(strategy);

        assert_eq!(runner.strategies.len(), 1);
        assert_eq!(runner.timer_manager.len(), 1);
    }

    #[tokio::test]
    async fn test_runner_shutdown() {
        let config = StrategyRunnerConfig::default();
        let runner = StrategyRunner::new(config);

        let (market_tx, market_rx) = mpsc::channel(10);
        let (execution_tx, execution_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Immediately signal shutdown
        shutdown_tx.send(true).unwrap();

        // Drop senders to close channels
        drop(market_tx);
        drop(execution_tx);

        // Runner should exit cleanly
        let result = runner.run(market_rx, execution_rx, shutdown_rx).await;
        assert!(result.is_ok());
    }
}
