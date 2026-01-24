use rand::Rng;
use std::time::Duration;

/// Exponential backoff with jitter for reconnection attempts.
///
/// Formula: min(max_delay, base * 2^attempt) + random_jitter
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    base: Duration,
    max_delay: Duration,
    jitter_factor: f64,
    attempt: u32,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter_factor: 0.1,
            attempt: 0,
        }
    }
}

impl ExponentialBackoff {
    /// Create a new ExponentialBackoff.
    ///
    /// # Arguments
    /// * `base` - Initial delay duration
    /// * `max_delay` - Maximum delay cap
    /// * `jitter_factor` - Jitter as a fraction of delay (0.0 to 1.0). Negative values are clamped to 0.
    pub fn new(base: Duration, max_delay: Duration, jitter_factor: f64) -> Self {
        Self {
            base,
            max_delay,
            // Clamp negative jitter to 0 to prevent gen_range panic
            jitter_factor: jitter_factor.max(0.0),
            attempt: 0,
        }
    }

    /// Calculate the next delay and increment the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let exp_delay = self.base.saturating_mul(2u32.saturating_pow(self.attempt));
        let capped_delay = exp_delay.min(self.max_delay);

        // Add jitter: random value in [-jitter_factor, +jitter_factor] of the delay
        let jitter_range = capped_delay.as_secs_f64() * self.jitter_factor;
        let jitter = if jitter_range > 0.0 {
            rand::thread_rng().gen_range(-jitter_range..=jitter_range)
        } else {
            0.0
        };
        let final_secs = (capped_delay.as_secs_f64() + jitter).max(0.0);

        self.attempt = self.attempt.saturating_add(1);

        Duration::from_secs_f64(final_secs)
    }

    /// Reset the attempt counter (call after a successful connection).
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Get current attempt number.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_increases_exponentially() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(1),
            Duration::from_secs(60),
            0.0, // No jitter for deterministic test
        );

        let d0 = backoff.next_delay();
        let d1 = backoff.next_delay();
        let d2 = backoff.next_delay();
        let d3 = backoff.next_delay();

        assert_eq!(d0, Duration::from_secs(1));
        assert_eq!(d1, Duration::from_secs(2));
        assert_eq!(d2, Duration::from_secs(4));
        assert_eq!(d3, Duration::from_secs(8));
    }

    #[test]
    fn test_backoff_caps_at_max() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_secs(1), Duration::from_secs(10), 0.0);

        // Exhaust attempts until we hit the cap
        for _ in 0..10 {
            backoff.next_delay();
        }

        let delay = backoff.next_delay();
        assert_eq!(delay, Duration::from_secs(10));
    }

    #[test]
    fn test_backoff_reset() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_secs(1), Duration::from_secs(60), 0.0);

        backoff.next_delay();
        backoff.next_delay();
        assert_eq!(backoff.attempt(), 2);

        backoff.reset();
        assert_eq!(backoff.attempt(), 0);

        let delay = backoff.next_delay();
        assert_eq!(delay, Duration::from_secs(1));
    }

    #[test]
    fn test_backoff_with_jitter_varies() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(10),
            Duration::from_secs(60),
            0.2, // 20% jitter
        );

        // With jitter, delays should vary but stay within bounds
        let delay = backoff.next_delay();
        let secs = delay.as_secs_f64();

        // Base is 10s, jitter is ±20%, so range is [8, 12]
        assert!(secs >= 8.0 && secs <= 12.0, "delay was {}", secs);
    }

    #[test]
    fn test_backoff_negative_jitter_clamped() {
        // Negative jitter should be clamped to 0, not panic
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(1),
            Duration::from_secs(60),
            -0.5, // Negative jitter
        );

        // Should not panic, and should return deterministic value (no jitter)
        let delay = backoff.next_delay();
        assert_eq!(delay, Duration::from_secs(1));
    }
}
