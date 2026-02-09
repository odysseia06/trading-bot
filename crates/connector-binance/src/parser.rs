use model::{DepthUpdate, Exchange, MakerSide, PriceLevelUpdate, Trade};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct BinanceTradeRaw {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "t")]
    pub trade_id: u64,
    #[serde(rename = "p")]
    pub price: Decimal,
    #[serde(rename = "q")]
    pub qty: Decimal,
    #[serde(rename = "T")]
    pub timestamp_ms: i64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

/// Raw Binance depth update event.
#[derive(Debug, Deserialize)]
pub struct BinanceDepthRaw {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub final_update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<(String, String)>,
    #[serde(rename = "a")]
    pub asks: Vec<(String, String)>,
}

/// Combined stream wrapper that holds generic event data.
#[derive(Debug, Deserialize)]
pub struct CombinedStreamWrapperGeneric {
    pub stream: String,
    pub data: serde_json::Value,
}

// Keep old wrapper for backwards compatibility in tests
#[derive(Debug, Deserialize)]
pub struct CombinedStreamWrapper {
    #[allow(dead_code)]
    pub stream: String,
    pub data: BinanceTradeRaw,
}

pub enum ParsedMessage {
    Trade(Trade),
    DepthUpdate(DepthUpdate),
    Unknown,
}

impl From<BinanceTradeRaw> for Trade {
    fn from(raw: BinanceTradeRaw) -> Self {
        Trade {
            exchange: Exchange::Binance,
            symbol: raw.symbol,
            price: raw.price,
            qty: raw.qty,
            trade_id: raw.trade_id,
            timestamp_ms: raw.timestamp_ms,
            maker_side: if raw.is_buyer_maker {
                MakerSide::Buyer
            } else {
                MakerSide::Seller
            },
        }
    }
}

/// Parse string price/qty pairs into Decimal tuples.
fn parse_price_levels(levels: &[(String, String)]) -> Vec<PriceLevelUpdate> {
    levels
        .iter()
        .filter_map(|(price, qty)| {
            let p = Decimal::from_str(price).ok()?;
            let q = Decimal::from_str(qty).ok()?;
            Some((p, q))
        })
        .collect()
}

impl From<BinanceDepthRaw> for DepthUpdate {
    fn from(raw: BinanceDepthRaw) -> Self {
        DepthUpdate {
            exchange: Exchange::Binance,
            symbol: raw.symbol,
            first_update_id: raw.first_update_id,
            final_update_id: raw.final_update_id,
            bids: parse_price_levels(&raw.bids),
            asks: parse_price_levels(&raw.asks),
            timestamp_ms: raw.event_time,
            is_snapshot: false, // WebSocket depth updates are always deltas
        }
    }
}

pub fn parse_message(text: &str) -> Result<ParsedMessage, serde_json::Error> {
    // Try combined stream format first (has "stream" field)
    if text.contains("\"stream\"") {
        let wrapper: CombinedStreamWrapperGeneric = serde_json::from_str(text)?;

        // Check event type from the data payload
        if let Some(event_type) = wrapper.data.get("e").and_then(|v| v.as_str()) {
            match event_type {
                "trade" => {
                    let trade_raw: BinanceTradeRaw = serde_json::from_value(wrapper.data)?;
                    return Ok(ParsedMessage::Trade(trade_raw.into()));
                }
                "depthUpdate" => {
                    let depth_raw: BinanceDepthRaw = serde_json::from_value(wrapper.data)?;
                    return Ok(ParsedMessage::DepthUpdate(depth_raw.into()));
                }
                _ => return Ok(ParsedMessage::Unknown),
            }
        }
        return Ok(ParsedMessage::Unknown);
    }

    // Try raw stream format (single subscription)
    let raw: serde_json::Value = serde_json::from_str(text)?;
    if let Some(event_type) = raw.get("e").and_then(|v| v.as_str()) {
        match event_type {
            "trade" => {
                let trade_raw: BinanceTradeRaw = serde_json::from_value(raw)?;
                return Ok(ParsedMessage::Trade(trade_raw.into()));
            }
            "depthUpdate" => {
                let depth_raw: BinanceDepthRaw = serde_json::from_value(raw)?;
                return Ok(ParsedMessage::DepthUpdate(depth_raw.into()));
            }
            _ => {}
        }
    }

    Ok(ParsedMessage::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_parse_raw_trade() {
        let json = r#"{
            "e": "trade",
            "E": 1672515782136,
            "s": "BTCUSDT",
            "t": 12345,
            "p": "23456.78",
            "q": "0.001",
            "b": 88,
            "a": 50,
            "T": 1672515782136,
            "m": true,
            "M": true
        }"#;

        let parsed = parse_message(json).unwrap();
        match parsed {
            ParsedMessage::Trade(trade) => {
                assert_eq!(trade.symbol, "BTCUSDT");
                assert_eq!(trade.trade_id, 12345);
                assert_eq!(trade.price.to_string(), "23456.78");
                assert_eq!(trade.qty.to_string(), "0.001");
                assert_eq!(trade.maker_side, MakerSide::Buyer);
            }
            _ => panic!("Expected Trade"),
        }
    }

    #[test]
    fn test_parse_combined_stream_trade() {
        let json = r#"{
            "stream": "btcusdt@trade",
            "data": {
                "e": "trade",
                "E": 1672515782136,
                "s": "BTCUSDT",
                "t": 12345,
                "p": "23456.78",
                "q": "0.001",
                "b": 88,
                "a": 50,
                "T": 1672515782136,
                "m": false,
                "M": true
            }
        }"#;

        let parsed = parse_message(json).unwrap();
        match parsed {
            ParsedMessage::Trade(trade) => {
                assert_eq!(trade.symbol, "BTCUSDT");
                assert_eq!(trade.maker_side, MakerSide::Seller);
            }
            _ => panic!("Expected Trade"),
        }
    }

    #[test]
    fn test_parse_raw_depth_update() {
        let json = r#"{
            "e": "depthUpdate",
            "E": 1672515782136,
            "s": "BTCUSDT",
            "U": 160,
            "u": 165,
            "b": [
                ["23450.00", "1.5"],
                ["23449.50", "2.0"]
            ],
            "a": [
                ["23455.00", "0.8"],
                ["23456.00", "0"]
            ]
        }"#;

        let parsed = parse_message(json).unwrap();
        match parsed {
            ParsedMessage::DepthUpdate(depth) => {
                assert_eq!(depth.symbol, "BTCUSDT");
                assert_eq!(depth.first_update_id, 160);
                assert_eq!(depth.final_update_id, 165);
                assert_eq!(depth.timestamp_ms, 1672515782136);

                // Check bids
                assert_eq!(depth.bids.len(), 2);
                assert_eq!(depth.bids[0], (dec!(23450.00), dec!(1.5)));
                assert_eq!(depth.bids[1], (dec!(23449.50), dec!(2.0)));

                // Check asks (including zero qty for removal)
                assert_eq!(depth.asks.len(), 2);
                assert_eq!(depth.asks[0], (dec!(23455.00), dec!(0.8)));
                assert_eq!(depth.asks[1], (dec!(23456.00), dec!(0)));
            }
            _ => panic!("Expected DepthUpdate"),
        }
    }

    #[test]
    fn test_parse_combined_stream_depth_update() {
        let json = r#"{
            "stream": "btcusdt@depth@100ms",
            "data": {
                "e": "depthUpdate",
                "E": 1672515782136,
                "s": "BTCUSDT",
                "U": 1000,
                "u": 1005,
                "b": [
                    ["50000.50", "0.1"]
                ],
                "a": [
                    ["50001.00", "0.2"]
                ]
            }
        }"#;

        let parsed = parse_message(json).unwrap();
        match parsed {
            ParsedMessage::DepthUpdate(depth) => {
                assert_eq!(depth.symbol, "BTCUSDT");
                assert_eq!(depth.first_update_id, 1000);
                assert_eq!(depth.final_update_id, 1005);
                assert_eq!(depth.bids.len(), 1);
                assert_eq!(depth.asks.len(), 1);
            }
            _ => panic!("Expected DepthUpdate"),
        }
    }

    #[test]
    fn test_parse_unknown_event() {
        let json = r#"{
            "e": "someOtherEvent",
            "E": 1672515782136,
            "s": "BTCUSDT"
        }"#;

        let parsed = parse_message(json).unwrap();
        assert!(matches!(parsed, ParsedMessage::Unknown));
    }
}
