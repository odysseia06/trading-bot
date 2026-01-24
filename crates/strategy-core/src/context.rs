//! Strategy context providing market state access.

use std::collections::HashMap;
use std::sync::Arc;

use orderbook::{OrderBook, PriceLevel};
use parking_lot::RwLock;
use rust_decimal::Decimal;

/// Market state shared between strategies and the runner.
pub struct MarketState {
    /// Order books indexed by symbol.
    order_books: RwLock<HashMap<String, OrderBook>>,
    /// Last traded prices indexed by symbol.
    last_prices: RwLock<HashMap<String, Decimal>>,
}

impl MarketState {
    /// Create a new empty market state.
    pub fn new() -> Self {
        Self {
            order_books: RwLock::new(HashMap::new()),
            last_prices: RwLock::new(HashMap::new()),
        }
    }

    /// Update or create an order book for a symbol.
    pub fn update_order_book(&self, symbol: &str, update_fn: impl FnOnce(&mut OrderBook)) {
        let mut books = self.order_books.write();
        let book = books
            .entry(symbol.to_string())
            .or_insert_with(|| OrderBook::new(symbol));
        update_fn(book);
    }

    /// Get a snapshot of the order book for a symbol.
    pub fn order_book(&self, symbol: &str) -> Option<OrderBookSnapshot> {
        let books = self.order_books.read();
        books.get(symbol).map(|book| OrderBookSnapshot {
            symbol: book.symbol().to_string(),
            best_bid: book.best_bid(),
            best_ask: book.best_ask(),
            mid_price: book.mid_price(),
            spread: book.spread(),
            bid_levels: book.bid_levels(),
            ask_levels: book.ask_levels(),
            last_update_id: book.last_update_id(),
        })
    }

    /// Get top N bid levels for a symbol.
    pub fn top_bids(&self, symbol: &str, n: usize) -> Vec<PriceLevel> {
        let books = self.order_books.read();
        books.get(symbol).map(|b| b.top_bids(n)).unwrap_or_default()
    }

    /// Get top N ask levels for a symbol.
    pub fn top_asks(&self, symbol: &str, n: usize) -> Vec<PriceLevel> {
        let books = self.order_books.read();
        books.get(symbol).map(|b| b.top_asks(n)).unwrap_or_default()
    }

    /// Update the last traded price for a symbol.
    pub fn update_last_price(&self, symbol: &str, price: Decimal) {
        let mut prices = self.last_prices.write();
        prices.insert(symbol.to_string(), price);
    }

    /// Get the last traded price for a symbol.
    pub fn last_price(&self, symbol: &str) -> Option<Decimal> {
        let prices = self.last_prices.read();
        prices.get(symbol).copied()
    }

    /// Get the best bid price for a symbol.
    pub fn best_bid(&self, symbol: &str) -> Option<Decimal> {
        let books = self.order_books.read();
        books
            .get(symbol)
            .and_then(|b| b.best_bid())
            .map(|l| l.price)
    }

    /// Get the best ask price for a symbol.
    pub fn best_ask(&self, symbol: &str) -> Option<Decimal> {
        let books = self.order_books.read();
        books
            .get(symbol)
            .and_then(|b| b.best_ask())
            .map(|l| l.price)
    }

    /// Get the mid price for a symbol.
    pub fn mid_price(&self, symbol: &str) -> Option<Decimal> {
        let books = self.order_books.read();
        books.get(symbol).and_then(|b| b.mid_price())
    }

    /// Get the spread for a symbol.
    pub fn spread(&self, symbol: &str) -> Option<Decimal> {
        let books = self.order_books.read();
        books.get(symbol).and_then(|b| b.spread())
    }
}

impl Default for MarketState {
    fn default() -> Self {
        Self::new()
    }
}

/// A read-only snapshot of an order book's state.
#[derive(Debug, Clone)]
pub struct OrderBookSnapshot {
    /// Trading pair symbol.
    pub symbol: String,
    /// Best bid level (highest price someone is willing to buy).
    pub best_bid: Option<PriceLevel>,
    /// Best ask level (lowest price someone is willing to sell).
    pub best_ask: Option<PriceLevel>,
    /// Mid price (average of best bid and best ask).
    pub mid_price: Option<Decimal>,
    /// Spread (best ask - best bid).
    pub spread: Option<Decimal>,
    /// Number of bid price levels.
    pub bid_levels: usize,
    /// Number of ask price levels.
    pub ask_levels: usize,
    /// Last update sequence ID.
    pub last_update_id: Option<u64>,
}

/// Shared market state handle.
pub type SharedMarketState = Arc<MarketState>;

/// Create a new shared market state.
pub fn create_market_state() -> SharedMarketState {
    Arc::new(MarketState::new())
}

/// Context provided to strategies during execution.
///
/// Provides read-only access to market state and current timestamp.
pub struct StrategyContext {
    /// Current timestamp in milliseconds since epoch.
    pub timestamp_ms: i64,
    /// Shared market state.
    market_state: SharedMarketState,
}

impl StrategyContext {
    /// Create a new strategy context.
    pub fn new(timestamp_ms: i64, market_state: SharedMarketState) -> Self {
        Self {
            timestamp_ms,
            market_state,
        }
    }

    /// Get a snapshot of the order book for a symbol.
    pub fn order_book(&self, symbol: &str) -> Option<OrderBookSnapshot> {
        self.market_state.order_book(symbol)
    }

    /// Get the last traded price for a symbol.
    pub fn last_price(&self, symbol: &str) -> Option<Decimal> {
        self.market_state.last_price(symbol)
    }

    /// Get the best bid price for a symbol.
    pub fn best_bid(&self, symbol: &str) -> Option<Decimal> {
        self.market_state.best_bid(symbol)
    }

    /// Get the best ask price for a symbol.
    pub fn best_ask(&self, symbol: &str) -> Option<Decimal> {
        self.market_state.best_ask(symbol)
    }

    /// Get the mid price for a symbol.
    pub fn mid_price(&self, symbol: &str) -> Option<Decimal> {
        self.market_state.mid_price(symbol)
    }

    /// Get the spread for a symbol.
    pub fn spread(&self, symbol: &str) -> Option<Decimal> {
        self.market_state.spread(symbol)
    }

    /// Get top N bid levels for a symbol.
    pub fn top_bids(&self, symbol: &str, n: usize) -> Vec<PriceLevel> {
        self.market_state.top_bids(symbol, n)
    }

    /// Get top N ask levels for a symbol.
    pub fn top_asks(&self, symbol: &str, n: usize) -> Vec<PriceLevel> {
        self.market_state.top_asks(symbol, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_market_state_last_price() {
        let state = MarketState::new();

        assert!(state.last_price("BTCUSDT").is_none());

        state.update_last_price("BTCUSDT", dec!(50000));
        assert_eq!(state.last_price("BTCUSDT"), Some(dec!(50000)));

        state.update_last_price("BTCUSDT", dec!(51000));
        assert_eq!(state.last_price("BTCUSDT"), Some(dec!(51000)));
    }

    #[test]
    fn test_market_state_order_book() {
        let state = MarketState::new();

        assert!(state.order_book("BTCUSDT").is_none());

        // Apply a snapshot
        state.update_order_book("BTCUSDT", |book| {
            let bids = vec![(dec!(50000), dec!(1.0))];
            let asks = vec![(dec!(50100), dec!(1.0))];
            book.apply_snapshot(&bids, &asks, 1000);
        });

        let snapshot = state.order_book("BTCUSDT").unwrap();
        assert_eq!(snapshot.best_bid.unwrap().price, dec!(50000));
        assert_eq!(snapshot.best_ask.unwrap().price, dec!(50100));
        assert_eq!(snapshot.mid_price, Some(dec!(50050)));
    }

    #[test]
    fn test_strategy_context() {
        let market_state = create_market_state();
        market_state.update_last_price("BTCUSDT", dec!(50000));

        let ctx = StrategyContext::new(1234567890, market_state);

        assert_eq!(ctx.timestamp_ms, 1234567890);
        assert_eq!(ctx.last_price("BTCUSDT"), Some(dec!(50000)));
        assert!(ctx.last_price("ETHUSDT").is_none());
    }

    #[test]
    fn test_top_levels() {
        let state = MarketState::new();

        state.update_order_book("BTCUSDT", |book| {
            let bids = vec![
                (dec!(100), dec!(1)),
                (dec!(99), dec!(2)),
                (dec!(98), dec!(3)),
            ];
            let asks = vec![
                (dec!(101), dec!(1)),
                (dec!(102), dec!(2)),
                (dec!(103), dec!(3)),
            ];
            book.apply_snapshot(&bids, &asks, 1);
        });

        let top_bids = state.top_bids("BTCUSDT", 2);
        assert_eq!(top_bids.len(), 2);
        assert_eq!(top_bids[0].price, dec!(100));
        assert_eq!(top_bids[1].price, dec!(99));

        let top_asks = state.top_asks("BTCUSDT", 2);
        assert_eq!(top_asks.len(), 2);
        assert_eq!(top_asks[0].price, dec!(101));
        assert_eq!(top_asks[1].price, dec!(102));
    }
}
