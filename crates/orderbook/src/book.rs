//! Order book implementation with sorted price levels.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use rust_decimal::Decimal;
use tracing::warn;

use crate::error::OrderBookError;
use crate::level::PriceLevel;

/// Local order book maintaining sorted bid and ask levels.
///
/// Uses `BTreeMap` with `Decimal` keys for precise price level tracking.
/// - Bids use `Reverse<Decimal>` for descending order (highest first)
/// - Asks use `Decimal` directly for ascending order (lowest first)
///
/// This ensures no precision loss compared to using `f64` keys.
#[derive(Debug)]
pub struct OrderBook {
    symbol: String,
    /// Bids stored with Reverse<Decimal> keys for descending order.
    /// Iteration yields highest price first.
    bids: BTreeMap<Reverse<Decimal>, Decimal>,
    /// Asks stored with Decimal keys for ascending order.
    /// Iteration yields lowest price first.
    asks: BTreeMap<Decimal, Decimal>,
    /// Last update ID for sequence tracking.
    last_update_id: Option<u64>,
    /// Whether the book has been initialized with a snapshot.
    initialized: bool,
}

impl OrderBook {
    /// Creates a new empty order book for the given symbol.
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_update_id: None,
            initialized: false,
        }
    }

    /// Returns the symbol this order book tracks.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Returns whether the order book has been initialized with a snapshot.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Returns the last update ID.
    pub fn last_update_id(&self) -> Option<u64> {
        self.last_update_id
    }

    /// Applies a full snapshot to the order book, replacing all existing data.
    pub fn apply_snapshot(
        &mut self,
        bids: &[(Decimal, Decimal)],
        asks: &[(Decimal, Decimal)],
        update_id: u64,
    ) {
        self.bids.clear();
        self.asks.clear();

        for (price, quantity) in bids {
            if !quantity.is_zero() {
                self.bids.insert(Reverse(*price), *quantity);
            }
        }

        for (price, quantity) in asks {
            if !quantity.is_zero() {
                self.asks.insert(*price, *quantity);
            }
        }

        self.last_update_id = Some(update_id);
        self.initialized = true;
    }

    /// Applies a delta update to the order book.
    ///
    /// Returns an error if a sequence gap is detected.
    pub fn apply_delta(
        &mut self,
        bids: &[(Decimal, Decimal)],
        asks: &[(Decimal, Decimal)],
        first_update_id: u64,
        final_update_id: u64,
    ) -> Result<(), OrderBookError> {
        if !self.initialized {
            return Err(OrderBookError::NotInitialized);
        }

        // Check for sequence gaps
        if let Some(last_id) = self.last_update_id {
            // For Binance: first_update_id should be <= last_update_id + 1
            // and final_update_id should be >= last_update_id + 1
            if first_update_id > last_id + 1 {
                warn!(
                    symbol = %self.symbol,
                    expected = last_id + 1,
                    got = first_update_id,
                    "sequence gap detected"
                );
                return Err(OrderBookError::SequenceGap {
                    expected: last_id + 1,
                    actual: first_update_id,
                });
            }

            // Skip stale updates
            if final_update_id <= last_id {
                return Ok(());
            }
        }

        // Apply bid updates
        for (price, quantity) in bids {
            if quantity.is_zero() {
                self.bids.remove(&Reverse(*price));
            } else {
                self.bids.insert(Reverse(*price), *quantity);
            }
        }

        // Apply ask updates
        for (price, quantity) in asks {
            if quantity.is_zero() {
                self.asks.remove(price);
            } else {
                self.asks.insert(*price, *quantity);
            }
        }

        self.last_update_id = Some(final_update_id);
        Ok(())
    }

    /// Returns the best (highest) bid price level.
    pub fn best_bid(&self) -> Option<PriceLevel> {
        self.bids
            .iter()
            .next()
            .map(|(Reverse(price), qty)| PriceLevel::new(*price, *qty))
    }

    /// Returns the best (lowest) ask price level.
    pub fn best_ask(&self) -> Option<PriceLevel> {
        self.asks
            .iter()
            .next()
            .map(|(price, qty)| PriceLevel::new(*price, *qty))
    }

    /// Returns the mid price (average of best bid and best ask).
    pub fn mid_price(&self) -> Option<Decimal> {
        let bid = self.best_bid()?;
        let ask = self.best_ask()?;
        Some((bid.price + ask.price) / Decimal::TWO)
    }

    /// Returns the spread (best ask - best bid).
    pub fn spread(&self) -> Option<Decimal> {
        let bid = self.best_bid()?;
        let ask = self.best_ask()?;
        Some(ask.price - bid.price)
    }

    /// Returns the spread as a percentage of the mid price.
    pub fn spread_bps(&self) -> Option<Decimal> {
        let spread = self.spread()?;
        let mid = self.mid_price()?;
        if mid.is_zero() {
            return None;
        }
        Some(spread / mid * Decimal::from(10000))
    }

    /// Returns the top N bid price levels (highest to lowest).
    pub fn top_bids(&self, n: usize) -> Vec<PriceLevel> {
        self.bids
            .iter()
            .take(n)
            .map(|(Reverse(price), qty)| PriceLevel::new(*price, *qty))
            .collect()
    }

    /// Returns the top N ask price levels (lowest to highest).
    pub fn top_asks(&self, n: usize) -> Vec<PriceLevel> {
        self.asks
            .iter()
            .take(n)
            .map(|(price, qty)| PriceLevel::new(*price, *qty))
            .collect()
    }

    /// Returns the total bid quantity up to and including the given price.
    /// (i.e., all bids at or above the given price)
    pub fn bid_depth_at(&self, price: Decimal) -> Decimal {
        // Bids are sorted descending (highest first via Reverse)
        // We want all bids >= price, which in Reverse order means all keys <= Reverse(price)
        self.bids.range(..=Reverse(price)).map(|(_, qty)| qty).sum()
    }

    /// Returns the total ask quantity up to and including the given price.
    /// (i.e., all asks at or below the given price)
    pub fn ask_depth_at(&self, price: Decimal) -> Decimal {
        self.asks.range(..=price).map(|(_, qty)| qty).sum()
    }

    /// Returns the total number of bid levels.
    pub fn bid_levels(&self) -> usize {
        self.bids.len()
    }

    /// Returns the total number of ask levels.
    pub fn ask_levels(&self) -> usize {
        self.asks.len()
    }

    /// Clears all data and resets the order book to uninitialized state.
    pub fn clear(&mut self) {
        self.bids.clear();
        self.asks.clear();
        self.last_update_id = None;
        self.initialized = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_empty_book() {
        let book = OrderBook::new("BTCUSDT");
        assert_eq!(book.symbol(), "BTCUSDT");
        assert!(!book.is_initialized());
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert!(book.mid_price().is_none());
        assert!(book.spread().is_none());
    }

    #[test]
    fn test_apply_snapshot() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![
            (dec!(100.0), dec!(1.0)),
            (dec!(99.0), dec!(2.0)),
            (dec!(98.0), dec!(3.0)),
        ];
        let asks = vec![
            (dec!(101.0), dec!(1.5)),
            (dec!(102.0), dec!(2.5)),
            (dec!(103.0), dec!(3.5)),
        ];

        book.apply_snapshot(&bids, &asks, 1000);

        assert!(book.is_initialized());
        assert_eq!(book.last_update_id(), Some(1000));

        let best_bid = book.best_bid().unwrap();
        assert_eq!(best_bid.price, dec!(100.0));
        assert_eq!(best_bid.quantity, dec!(1.0));

        let best_ask = book.best_ask().unwrap();
        assert_eq!(best_ask.price, dec!(101.0));
        assert_eq!(best_ask.quantity, dec!(1.5));

        assert_eq!(book.mid_price(), Some(dec!(100.5)));
        assert_eq!(book.spread(), Some(dec!(1.0)));
    }

    #[test]
    fn test_top_levels() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![
            (dec!(100.0), dec!(1.0)),
            (dec!(99.0), dec!(2.0)),
            (dec!(98.0), dec!(3.0)),
        ];
        let asks = vec![
            (dec!(101.0), dec!(1.5)),
            (dec!(102.0), dec!(2.5)),
            (dec!(103.0), dec!(3.5)),
        ];

        book.apply_snapshot(&bids, &asks, 1000);

        let top_bids = book.top_bids(2);
        assert_eq!(top_bids.len(), 2);
        assert_eq!(top_bids[0].price, dec!(100.0));
        assert_eq!(top_bids[1].price, dec!(99.0));

        let top_asks = book.top_asks(2);
        assert_eq!(top_asks.len(), 2);
        assert_eq!(top_asks[0].price, dec!(101.0));
        assert_eq!(top_asks[1].price, dec!(102.0));
    }

    #[test]
    fn test_apply_delta() {
        let mut book = OrderBook::new("BTCUSDT");

        // Initial snapshot
        let bids = vec![(dec!(100.0), dec!(1.0))];
        let asks = vec![(dec!(101.0), dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        // Delta: update bid quantity, add new ask
        let delta_bids = vec![(dec!(100.0), dec!(2.0))];
        let delta_asks = vec![(dec!(100.5), dec!(0.5))];
        book.apply_delta(&delta_bids, &delta_asks, 1001, 1001)
            .unwrap();

        assert_eq!(book.best_bid().unwrap().quantity, dec!(2.0));
        assert_eq!(book.best_ask().unwrap().price, dec!(100.5));
        assert_eq!(book.last_update_id(), Some(1001));
    }

    #[test]
    fn test_delta_removes_zero_quantity() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![(dec!(100.0), dec!(1.0)), (dec!(99.0), dec!(2.0))];
        let asks = vec![(dec!(101.0), dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        // Remove the best bid by setting quantity to zero
        let delta_bids = vec![(dec!(100.0), dec!(0.0))];
        book.apply_delta(&delta_bids, &[], 1001, 1001).unwrap();

        // New best bid should be 99.0
        assert_eq!(book.best_bid().unwrap().price, dec!(99.0));
        assert_eq!(book.bid_levels(), 1);
    }

    #[test]
    fn test_sequence_gap_detection() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![(dec!(100.0), dec!(1.0))];
        let asks = vec![(dec!(101.0), dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        // Try to apply delta with a gap (1000 + 1 = 1001, but we're sending 1003)
        let result = book.apply_delta(&[], &[], 1003, 1003);
        assert!(matches!(result, Err(OrderBookError::SequenceGap { .. })));
    }

    #[test]
    fn test_stale_update_ignored() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![(dec!(100.0), dec!(1.0))];
        let asks = vec![(dec!(101.0), dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        // Apply valid update
        book.apply_delta(&[(dec!(100.0), dec!(2.0))], &[], 1001, 1001)
            .unwrap();
        assert_eq!(book.best_bid().unwrap().quantity, dec!(2.0));

        // Try to apply stale update (final_update_id <= last_update_id)
        book.apply_delta(&[(dec!(100.0), dec!(5.0))], &[], 999, 1000)
            .unwrap();
        // Should be ignored, quantity unchanged
        assert_eq!(book.best_bid().unwrap().quantity, dec!(2.0));
    }

    #[test]
    fn test_delta_without_snapshot_fails() {
        let mut book = OrderBook::new("BTCUSDT");

        let result = book.apply_delta(&[], &[], 1, 1);
        assert!(matches!(result, Err(OrderBookError::NotInitialized)));
    }

    #[test]
    fn test_spread_bps() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![(dec!(100.0), dec!(1.0))];
        let asks = vec![(dec!(101.0), dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        // Spread = 1.0, mid = 100.5
        // Spread bps = 1.0 / 100.5 * 10000 ≈ 99.50
        let spread_bps = book.spread_bps().unwrap();
        assert!(spread_bps > dec!(99) && spread_bps < dec!(100));
    }

    #[test]
    fn test_depth_at_price() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![
            (dec!(100.0), dec!(1.0)),
            (dec!(99.0), dec!(2.0)),
            (dec!(98.0), dec!(3.0)),
        ];
        let asks = vec![
            (dec!(101.0), dec!(1.0)),
            (dec!(102.0), dec!(2.0)),
            (dec!(103.0), dec!(3.0)),
        ];
        book.apply_snapshot(&bids, &asks, 1000);

        // Bid depth at 99.0 should include 100.0 and 99.0 (bids >= 99.0)
        assert_eq!(book.bid_depth_at(dec!(99.0)), dec!(3.0));

        // Ask depth at 102.0 should include 101.0 and 102.0 (asks <= 102.0)
        assert_eq!(book.ask_depth_at(dec!(102.0)), dec!(3.0));
    }

    #[test]
    fn test_clear() {
        let mut book = OrderBook::new("BTCUSDT");

        let bids = vec![(dec!(100.0), dec!(1.0))];
        let asks = vec![(dec!(101.0), dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        assert!(book.is_initialized());

        book.clear();

        assert!(!book.is_initialized());
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert!(book.last_update_id().is_none());
    }

    #[test]
    fn test_high_precision_prices_preserved() {
        // Test that prices with many decimal places are preserved exactly
        let mut book = OrderBook::new("BTCUSDT");

        // Use prices that would lose precision in f64 (f64 has ~15-17 significant digits)
        // These differ in the 20th+ significant digit, beyond f64's precision
        let precise_bid = dec!(100.1234567890123456789);
        let precise_ask = dec!(100.1234567890123456790); // Differs in last digit

        let bids = vec![(precise_bid, dec!(1.0))];
        let asks = vec![(precise_ask, dec!(1.0))];
        book.apply_snapshot(&bids, &asks, 1000);

        // Both prices should be preserved exactly
        assert_eq!(book.best_bid().unwrap().price, precise_bid);
        assert_eq!(book.best_ask().unwrap().price, precise_ask);

        // They should be distinct (not collapsed)
        assert_ne!(
            book.best_bid().unwrap().price,
            book.best_ask().unwrap().price
        );
    }

    #[test]
    fn test_prices_differing_in_low_digits_stay_distinct() {
        // Prices that would map to same f64 but are distinct Decimals
        let mut book = OrderBook::new("BTCUSDT");

        let price1 = dec!(0.00000001);
        let price2 = dec!(0.00000002);
        let price3 = dec!(0.00000003);

        let bids = vec![
            (price3, dec!(3.0)),
            (price2, dec!(2.0)),
            (price1, dec!(1.0)),
        ];
        book.apply_snapshot(&bids, &[], 1000);

        // Should have 3 distinct levels
        assert_eq!(book.bid_levels(), 3);

        let top = book.top_bids(3);
        assert_eq!(top[0].price, price3);
        assert_eq!(top[1].price, price2);
        assert_eq!(top[2].price, price1);
    }
}
