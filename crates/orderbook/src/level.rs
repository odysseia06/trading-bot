//! Price level representation.

use rust_decimal::Decimal;

/// A single price level in the order book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceLevel {
    /// The price at this level.
    pub price: Decimal,
    /// The total quantity available at this price.
    pub quantity: Decimal,
}

impl PriceLevel {
    /// Creates a new price level.
    pub fn new(price: Decimal, quantity: Decimal) -> Self {
        Self { price, quantity }
    }

    /// Returns the notional value (price * quantity) at this level.
    pub fn notional(&self) -> Decimal {
        self.price * self.quantity
    }
}
