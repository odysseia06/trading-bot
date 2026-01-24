use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Thread-safe metrics collector for the market connector.
#[derive(Debug)]
pub struct ConnectorMetrics {
    // Counters
    trades_received: AtomicU64,
    messages_received: AtomicU64,
    parse_errors: AtomicU64,
    websocket_errors: AtomicU64,
    reconnect_attempts: AtomicU64,
    reconnect_successes: AtomicU64,

    // Timestamps
    inner: RwLock<MetricsInner>,
}

#[derive(Debug)]
struct MetricsInner {
    start_time: Instant,
    last_trade_time: Option<Instant>,
    last_error_time: Option<Instant>,
    last_reconnect_time: Option<Instant>,
}

impl Default for ConnectorMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectorMetrics {
    pub fn new() -> Self {
        Self {
            trades_received: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            parse_errors: AtomicU64::new(0),
            websocket_errors: AtomicU64::new(0),
            reconnect_attempts: AtomicU64::new(0),
            reconnect_successes: AtomicU64::new(0),
            inner: RwLock::new(MetricsInner {
                start_time: Instant::now(),
                last_trade_time: None,
                last_error_time: None,
                last_reconnect_time: None,
            }),
        }
    }

    // --- Increment methods ---

    pub fn inc_trades_received(&self) {
        self.trades_received.fetch_add(1, Ordering::Relaxed);
        self.inner.write().last_trade_time = Some(Instant::now());
    }

    pub fn inc_messages_received(&self) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_parse_errors(&self) {
        self.parse_errors.fetch_add(1, Ordering::Relaxed);
        self.inner.write().last_error_time = Some(Instant::now());
    }

    pub fn inc_websocket_errors(&self) {
        self.websocket_errors.fetch_add(1, Ordering::Relaxed);
        self.inner.write().last_error_time = Some(Instant::now());
    }

    pub fn inc_reconnect_attempts(&self) {
        self.reconnect_attempts.fetch_add(1, Ordering::Relaxed);
        self.inner.write().last_reconnect_time = Some(Instant::now());
    }

    pub fn inc_reconnect_successes(&self) {
        self.reconnect_successes.fetch_add(1, Ordering::Relaxed);
    }

    // --- Getter methods ---

    pub fn trades_received(&self) -> u64 {
        self.trades_received.load(Ordering::Relaxed)
    }

    pub fn messages_received(&self) -> u64 {
        self.messages_received.load(Ordering::Relaxed)
    }

    pub fn parse_errors(&self) -> u64 {
        self.parse_errors.load(Ordering::Relaxed)
    }

    pub fn websocket_errors(&self) -> u64 {
        self.websocket_errors.load(Ordering::Relaxed)
    }

    pub fn reconnect_attempts(&self) -> u64 {
        self.reconnect_attempts.load(Ordering::Relaxed)
    }

    pub fn reconnect_successes(&self) -> u64 {
        self.reconnect_successes.load(Ordering::Relaxed)
    }

    pub fn uptime_secs(&self) -> f64 {
        self.inner.read().start_time.elapsed().as_secs_f64()
    }

    pub fn secs_since_last_trade(&self) -> Option<f64> {
        self.inner
            .read()
            .last_trade_time
            .map(|t| t.elapsed().as_secs_f64())
    }

    pub fn secs_since_last_error(&self) -> Option<f64> {
        self.inner
            .read()
            .last_error_time
            .map(|t| t.elapsed().as_secs_f64())
    }

    /// Calculate trades per second since start.
    pub fn trades_per_second(&self) -> f64 {
        let uptime = self.uptime_secs();
        if uptime > 0.0 {
            self.trades_received() as f64 / uptime
        } else {
            0.0
        }
    }

    /// Generate a snapshot of all metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            trades_received: self.trades_received(),
            messages_received: self.messages_received(),
            parse_errors: self.parse_errors(),
            websocket_errors: self.websocket_errors(),
            reconnect_attempts: self.reconnect_attempts(),
            reconnect_successes: self.reconnect_successes(),
            uptime_secs: self.uptime_secs(),
            trades_per_second: self.trades_per_second(),
            secs_since_last_trade: self.secs_since_last_trade(),
            secs_since_last_error: self.secs_since_last_error(),
        }
    }
}

/// A point-in-time snapshot of metrics.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub trades_received: u64,
    pub messages_received: u64,
    pub parse_errors: u64,
    pub websocket_errors: u64,
    pub reconnect_attempts: u64,
    pub reconnect_successes: u64,
    pub uptime_secs: f64,
    pub trades_per_second: f64,
    pub secs_since_last_trade: Option<f64>,
    pub secs_since_last_error: Option<f64>,
}

/// Health status of the connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Connector is healthy and receiving data.
    Healthy,
    /// Connector is degraded (e.g., high error rate or stale data).
    Degraded,
    /// Connector is unhealthy (no data for extended period).
    Unhealthy,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "HEALTHY"),
            HealthStatus::Degraded => write!(f, "DEGRADED"),
            HealthStatus::Unhealthy => write!(f, "UNHEALTHY"),
        }
    }
}

impl MetricsSnapshot {
    /// Threshold in seconds for considering data stale (degraded).
    const STALE_THRESHOLD_SECS: f64 = 30.0;
    /// Threshold in seconds for considering connector unhealthy.
    const UNHEALTHY_THRESHOLD_SECS: f64 = 60.0;

    /// Determine the health status based on metrics.
    pub fn health_status(&self) -> HealthStatus {
        // If we've never received a trade, check uptime
        let secs_since_trade = match self.secs_since_last_trade {
            Some(secs) => secs,
            None => {
                // No trades yet - if uptime is short, we're still starting up
                if self.uptime_secs < Self::STALE_THRESHOLD_SECS {
                    return HealthStatus::Healthy;
                } else if self.uptime_secs < Self::UNHEALTHY_THRESHOLD_SECS {
                    return HealthStatus::Degraded;
                } else {
                    return HealthStatus::Unhealthy;
                }
            }
        };

        if secs_since_trade > Self::UNHEALTHY_THRESHOLD_SECS {
            HealthStatus::Unhealthy
        } else if secs_since_trade > Self::STALE_THRESHOLD_SECS {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        }
    }
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Connector Metrics ===")?;
        writeln!(f, "Uptime:              {:.1}s", self.uptime_secs)?;
        writeln!(f, "Trades received:     {}", self.trades_received)?;
        writeln!(f, "Messages received:   {}", self.messages_received)?;
        writeln!(f, "Trades/sec:          {:.2}", self.trades_per_second)?;
        writeln!(f, "Parse errors:        {}", self.parse_errors)?;
        writeln!(f, "WebSocket errors:    {}", self.websocket_errors)?;
        writeln!(f, "Reconnect attempts:  {}", self.reconnect_attempts)?;
        writeln!(f, "Reconnect successes: {}", self.reconnect_successes)?;
        if let Some(secs) = self.secs_since_last_trade {
            writeln!(f, "Since last trade:    {:.1}s", secs)?;
        }
        if let Some(secs) = self.secs_since_last_error {
            writeln!(f, "Since last error:    {:.1}s", secs)?;
        }
        Ok(())
    }
}

/// Shared handle to metrics.
pub type SharedMetrics = Arc<ConnectorMetrics>;

pub fn create_metrics() -> SharedMetrics {
    Arc::new(ConnectorMetrics::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_increment() {
        let metrics = ConnectorMetrics::new();

        metrics.inc_trades_received();
        metrics.inc_trades_received();
        metrics.inc_messages_received();
        metrics.inc_parse_errors();

        assert_eq!(metrics.trades_received(), 2);
        assert_eq!(metrics.messages_received(), 1);
        assert_eq!(metrics.parse_errors(), 1);
    }

    #[test]
    fn test_metrics_snapshot() {
        let metrics = ConnectorMetrics::new();

        metrics.inc_trades_received();
        metrics.inc_websocket_errors();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.trades_received, 1);
        assert_eq!(snapshot.websocket_errors, 1);
        assert!(snapshot.uptime_secs >= 0.0);
    }

    #[test]
    fn test_last_trade_time() {
        let metrics = ConnectorMetrics::new();

        assert!(metrics.secs_since_last_trade().is_none());

        metrics.inc_trades_received();

        let secs = metrics.secs_since_last_trade();
        assert!(secs.is_some());
        assert!(secs.unwrap() < 1.0);
    }

    // HealthStatus boundary tests

    #[test]
    fn test_health_status_healthy_with_recent_trade() {
        let snapshot = MetricsSnapshot {
            trades_received: 100,
            messages_received: 100,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 120.0,
            trades_per_second: 0.83,
            secs_since_last_trade: Some(5.0), // Recent trade
            secs_since_last_error: None,
        };

        assert_eq!(snapshot.health_status(), HealthStatus::Healthy);
    }

    #[test]
    fn test_health_status_healthy_during_startup() {
        // No trades yet, but uptime is short (still starting up)
        let snapshot = MetricsSnapshot {
            trades_received: 0,
            messages_received: 0,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 10.0, // Short uptime
            trades_per_second: 0.0,
            secs_since_last_trade: None, // No trades yet
            secs_since_last_error: None,
        };

        assert_eq!(snapshot.health_status(), HealthStatus::Healthy);
    }

    #[test]
    fn test_health_status_degraded_stale_data() {
        let snapshot = MetricsSnapshot {
            trades_received: 100,
            messages_received: 100,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 120.0,
            trades_per_second: 0.83,
            secs_since_last_trade: Some(45.0), // Between 30s and 60s
            secs_since_last_error: None,
        };

        assert_eq!(snapshot.health_status(), HealthStatus::Degraded);
    }

    #[test]
    fn test_health_status_degraded_no_trades_medium_uptime() {
        // No trades and uptime between 30s and 60s
        let snapshot = MetricsSnapshot {
            trades_received: 0,
            messages_received: 0,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 45.0, // Between thresholds
            trades_per_second: 0.0,
            secs_since_last_trade: None,
            secs_since_last_error: None,
        };

        assert_eq!(snapshot.health_status(), HealthStatus::Degraded);
    }

    #[test]
    fn test_health_status_unhealthy_very_stale_data() {
        let snapshot = MetricsSnapshot {
            trades_received: 100,
            messages_received: 100,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 300.0,
            trades_per_second: 0.33,
            secs_since_last_trade: Some(90.0), // Over 60s
            secs_since_last_error: None,
        };

        assert_eq!(snapshot.health_status(), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_health_status_unhealthy_no_trades_long_uptime() {
        // No trades ever and uptime over 60s
        let snapshot = MetricsSnapshot {
            trades_received: 0,
            messages_received: 0,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 120.0, // Long uptime
            trades_per_second: 0.0,
            secs_since_last_trade: None, // Never received trades
            secs_since_last_error: None,
        };

        assert_eq!(snapshot.health_status(), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_health_status_boundary_at_30_seconds() {
        // Exactly at 30s boundary - should still be healthy
        let snapshot = MetricsSnapshot {
            trades_received: 100,
            messages_received: 100,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 120.0,
            trades_per_second: 0.83,
            secs_since_last_trade: Some(30.0), // Exactly at threshold
            secs_since_last_error: None,
        };

        // At exactly 30s, it's not > 30, so still healthy
        assert_eq!(snapshot.health_status(), HealthStatus::Healthy);
    }

    #[test]
    fn test_health_status_boundary_at_60_seconds() {
        // Exactly at 60s boundary - should be degraded, not unhealthy
        let snapshot = MetricsSnapshot {
            trades_received: 100,
            messages_received: 100,
            parse_errors: 0,
            websocket_errors: 0,
            reconnect_attempts: 0,
            reconnect_successes: 0,
            uptime_secs: 120.0,
            trades_per_second: 0.83,
            secs_since_last_trade: Some(60.0), // Exactly at threshold
            secs_since_last_error: None,
        };

        // At exactly 60s, it's not > 60, so degraded
        assert_eq!(snapshot.health_status(), HealthStatus::Degraded);
    }
}
