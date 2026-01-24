//! Order book implementation with sorted price levels.

use std::collections::BTreeMap;

use ordered_float::OrderedFloat;
use rust_decimal::Decimal;
use tracing::warn;

use crate::error::OrderBookError;
use crate::level::PriceLevel;

/// Local order book maintaining sorted bid and ask levels.
///
/// Uses `BTreeMap` with `OrderedFloat` keys for efficient sorted access.
/// Bids are stored with negated keys for descending order iteration.
#[derive(Debug)]
pub struct OrderBook {
    symbol: String,
    /// Bids stored with negated price keys for descending order.
    /// Key: -price (so iteration is highest to lowest)
    bids: BTreeMap<OrderedFloat<f64>, Decimal>,
    /// Asks stored with normal price keys for ascending order.
    asks: BTreeMap<OrderedFloat<f64>, Decimal>,
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
                let key = Self::bid_key(*price);
                self.bids.insert(key, *quantity);
            }
        }

        for (price, quantity) in asks {
            if !quantity.is_zero() {
                let key = Self::ask_key(*price);
                self.asks.insert(key, *quantity);
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
            let key = Self::bid_key(*price);
            if quantity.is_zero() {
                self.bids.remove(&key);
            } else {
                self.bids.insert(key, *quantity);
            }
        }

        // Apply ask updates
        for (price, quantity) in asks {
            let key = Self::ask_key(*price);
            if quantity.is_zero() {
                self.asks.remove(&key);
            } else {
                self.asks.insert(key, *quantity);
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
            .map(|(key, qty)| PriceLevel::new(Self::price_from_bid_key(*key), *qty))
    }

    /// Returns the best (lowest) ask price level.
    pub fn best_ask(&self) -> Option<PriceLevel> {
        self.asks
            .iter()
            .next()
            .map(|(key, qty)| PriceLevel::new(Self::price_from_ask_key(*key), *qty))
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
            .map(|(key, qty)| PriceLevel::new(Self::price_from_bid_key(*key), *qty))
            .collect()
    }

    /// Returns the top N ask price levels (lowest to highest).
    pub fn top_asks(&self, n: usize) -> Vec<PriceLevel> {
        self.asks
            .iter()
            .take(n)
            .map(|(key, qty)| PriceLevel::new(Self::price_from_ask_key(*key), *qty))
            .collect()
    }

    /// Returns the total bid quantity up to and including the given price.
    pub fn bid_depth_at(&self, price: Decimal) -> Decimal {
        let target_key = Self::bid_key(price);
        self.bids.range(..=target_key).map(|(_, qty)| qty).sum()
    }

    /// Returns the total ask quantity up to and including the given price.
    pub fn ask_depth_at(&self, price: Decimal) -> Decimal {
        let target_key = Self::ask_key(price);
        self.asks.range(..=target_key).map(|(_, qty)| qty).sum()
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

    // Helper to convert price to bid key (negated for descending order)
    fn bid_key(price: Decimal) -> OrderedFloat<f64> {
        use rust_decimal::prelude::ToPrimitive;
        OrderedFloat(-price.to_f64().unwrap_or(0.0))
    }

    // Helper to convert price to ask key (normal for ascending order)
    fn ask_key(price: Decimal) -> OrderedFloat<f64> {
        use rust_decimal::prelude::ToPrimitive;
        OrderedFloat(price.to_f64().unwrap_or(0.0))
    }

    // Helper to convert bid key back to price
    fn price_from_bid_key(key: OrderedFloat<f64>) -> Decimal {
        Decimal::try_from(-key.0).unwrap_or_default()
    }

    // Helper to convert ask key back to price
    fn price_from_ask_key(key: OrderedFloat<f64>) -> Decimal {
        Decimal::try_from(key.0).unwrap_or_default()
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

        // Bid depth at 99.0 should include 100.0 and 99.0 (due to negated keys)
        assert_eq!(book.bid_depth_at(dec!(99.0)), dec!(3.0));

        // Ask depth at 102.0 should include 101.0 and 102.0
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
}
