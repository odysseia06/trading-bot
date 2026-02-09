use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Exchange {
    Binance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MakerSide {
    Buyer,
    Seller,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub exchange: Exchange,
    pub symbol: String,
    pub price: Decimal,
    pub qty: Decimal,
    pub trade_id: u64,
    pub timestamp_ms: i64,
    pub maker_side: MakerSide,
}

/// A price level update: (price, quantity).
/// Quantity of 0 means remove the level.
pub type PriceLevelUpdate = (Decimal, Decimal);

/// Depth update event from exchange order book.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthUpdate {
    pub exchange: Exchange,
    pub symbol: String,
    /// First update ID in this event (for sequence validation).
    pub first_update_id: u64,
    /// Final update ID in this event.
    pub final_update_id: u64,
    /// Bid updates: [(price, qty), ...]. Qty=0 means remove level.
    pub bids: Vec<PriceLevelUpdate>,
    /// Ask updates: [(price, qty), ...]. Qty=0 means remove level.
    pub asks: Vec<PriceLevelUpdate>,
    /// Event timestamp from exchange.
    pub timestamp_ms: i64,
    /// True if this is a full snapshot, false if it's a delta update.
    #[serde(default)]
    pub is_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketEvent {
    Trade(Trade),
    DepthUpdate(DepthUpdate),
}
