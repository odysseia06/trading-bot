//! Timer management for strategy periodic callbacks.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Tracks timer state for multiple strategies.
pub struct TimerManager {
    /// Strategy ID -> (interval, last_fired)
    timers: HashMap<String, (Duration, Instant)>,
}

impl TimerManager {
    /// Create a new timer manager.
    pub fn new() -> Self {
        Self {
            timers: HashMap::new(),
        }
    }

    /// Register a timer for a strategy.
    pub fn register(&mut self, strategy_id: impl Into<String>, interval: Duration) {
        self.timers
            .insert(strategy_id.into(), (interval, Instant::now()));
    }

    /// Remove a timer for a strategy.
    pub fn unregister(&mut self, strategy_id: &str) {
        self.timers.remove(strategy_id);
    }

    /// Check which strategy timers are due and return their IDs.
    ///
    /// Updates the last_fired time for each fired timer.
    pub fn check_due(&mut self) -> Vec<String> {
        let now = Instant::now();
        let mut due = Vec::new();

        for (strategy_id, (interval, last_fired)) in &mut self.timers {
            if now.duration_since(*last_fired) >= *interval {
                due.push(strategy_id.clone());
                *last_fired = now;
            }
        }

        due
    }

    /// Returns the minimum time until the next timer fires.
    ///
    /// Returns `None` if no timers are registered.
    pub fn next_deadline(&self) -> Option<Duration> {
        let now = Instant::now();

        self.timers
            .values()
            .map(|(interval, last_fired)| {
                let elapsed = now.duration_since(*last_fired);
                if elapsed >= *interval {
                    Duration::ZERO
                } else {
                    *interval - elapsed
                }
            })
            .min()
    }

    /// Returns the number of registered timers.
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// Returns whether any timers are registered.
    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }
}

impl Default for TimerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_manager() {
        let manager = TimerManager::new();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
        assert!(manager.next_deadline().is_none());
    }

    #[test]
    fn test_register_timer() {
        let mut manager = TimerManager::new();
        manager.register("strategy_1", Duration::from_secs(5));

        assert!(!manager.is_empty());
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn test_unregister_timer() {
        let mut manager = TimerManager::new();
        manager.register("strategy_1", Duration::from_secs(5));
        manager.unregister("strategy_1");

        assert!(manager.is_empty());
    }

    #[test]
    fn test_check_due_not_yet() {
        let mut manager = TimerManager::new();
        manager.register("strategy_1", Duration::from_secs(5));

        // Should not be due immediately
        let due = manager.check_due();
        assert!(due.is_empty());
    }

    #[test]
    fn test_next_deadline() {
        let mut manager = TimerManager::new();
        manager.register("strategy_1", Duration::from_secs(5));
        manager.register("strategy_2", Duration::from_secs(10));

        // Next deadline should be close to 5 seconds (the shorter interval)
        let deadline = manager.next_deadline().unwrap();
        assert!(deadline <= Duration::from_secs(5));
    }

    #[test]
    fn test_multiple_timers() {
        let mut manager = TimerManager::new();
        manager.register("strategy_1", Duration::from_millis(1));
        manager.register("strategy_2", Duration::from_secs(60));

        // Wait a tiny bit for the first timer to fire
        std::thread::sleep(Duration::from_millis(2));

        let due = manager.check_due();
        assert!(due.contains(&"strategy_1".to_string()));
        assert!(!due.contains(&"strategy_2".to_string()));
    }
}
