use model::{Exchange, MakerSide, Trade};
use rust_decimal::Decimal;
use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
pub struct CombinedStreamWrapper {
    #[allow(dead_code)]
    pub stream: String,
    pub data: BinanceTradeRaw,
}

pub enum ParsedMessage {
    Trade(Trade),
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

pub fn parse_message(text: &str) -> Result<ParsedMessage, serde_json::Error> {
    // Try combined stream format first (has "stream" field)
    if text.contains("\"stream\"") {
        let wrapper: CombinedStreamWrapper = serde_json::from_str(text)?;
        if wrapper.data.event_type == "trade" {
            return Ok(ParsedMessage::Trade(wrapper.data.into()));
        }
        return Ok(ParsedMessage::Unknown);
    }

    // Try raw stream format
    let raw: serde_json::Value = serde_json::from_str(text)?;
    if let Some(event_type) = raw.get("e").and_then(|v| v.as_str()) {
        if event_type == "trade" {
            let trade_raw: BinanceTradeRaw = serde_json::from_value(raw)?;
            return Ok(ParsedMessage::Trade(trade_raw.into()));
        }
    }

    Ok(ParsedMessage::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
