//! Binance API response types.

use rust_decimal::Decimal;
use serde::Deserialize;

/// Response from GET /api/v3/time.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerTimeResponse {
    #[serde(rename = "serverTime")]
    pub server_time: i64,
}

/// Response from POST /api/v3/userDataStream.
#[derive(Debug, Clone, Deserialize)]
pub struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    pub listen_key: String,
}

/// Response from POST /api/v3/order.
#[derive(Debug, Clone, Deserialize)]
pub struct NewOrderResponse {
    pub symbol: String,
    #[serde(rename = "orderId")]
    pub order_id: u64,
    #[serde(rename = "orderListId")]
    pub order_list_id: i64,
    #[serde(rename = "clientOrderId")]
    pub client_order_id: String,
    #[serde(rename = "transactTime")]
    pub transact_time: i64,
    #[serde(deserialize_with = "deserialize_decimal_from_str")]
    pub price: Decimal,
    #[serde(rename = "origQty", deserialize_with = "deserialize_decimal_from_str")]
    pub orig_qty: Decimal,
    #[serde(
        rename = "executedQty",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub executed_qty: Decimal,
    #[serde(
        rename = "cummulativeQuoteQty",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub cummulative_quote_qty: Decimal,
    pub status: String,
    #[serde(rename = "timeInForce")]
    pub time_in_force: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub side: String,
    /// Fills included when using newOrderRespType=FULL
    #[serde(default)]
    pub fills: Vec<OrderFill>,
}

/// A fill from an order response.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderFill {
    #[serde(deserialize_with = "deserialize_decimal_from_str")]
    pub price: Decimal,
    #[serde(deserialize_with = "deserialize_decimal_from_str")]
    pub qty: Decimal,
    #[serde(deserialize_with = "deserialize_decimal_from_str")]
    pub commission: Decimal,
    #[serde(rename = "commissionAsset")]
    pub commission_asset: String,
    #[serde(rename = "tradeId")]
    pub trade_id: u64,
}

/// Response from GET /api/v3/order.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderQueryResponse {
    pub symbol: String,
    #[serde(rename = "orderId")]
    pub order_id: u64,
    #[serde(rename = "orderListId")]
    pub order_list_id: i64,
    #[serde(rename = "clientOrderId")]
    pub client_order_id: String,
    #[serde(deserialize_with = "deserialize_decimal_from_str")]
    pub price: Decimal,
    #[serde(rename = "origQty", deserialize_with = "deserialize_decimal_from_str")]
    pub orig_qty: Decimal,
    #[serde(
        rename = "executedQty",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub executed_qty: Decimal,
    #[serde(
        rename = "cummulativeQuoteQty",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub cummulative_quote_qty: Decimal,
    pub status: String,
    #[serde(rename = "timeInForce")]
    pub time_in_force: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub side: String,
    #[serde(
        rename = "stopPrice",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub stop_price: Decimal,
    #[serde(
        rename = "icebergQty",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub iceberg_qty: Decimal,
    pub time: i64,
    #[serde(rename = "updateTime")]
    pub update_time: i64,
    #[serde(rename = "isWorking")]
    pub is_working: bool,
    #[serde(
        rename = "origQuoteOrderQty",
        deserialize_with = "deserialize_decimal_from_str"
    )]
    pub orig_quote_order_qty: Decimal,
}

/// Deserialize a Decimal from a string.
fn deserialize_decimal_from_str<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    s.parse::<Decimal>().map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_server_time() {
        let json = r#"{"serverTime": 1499827319559}"#;
        let response: ServerTimeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.server_time, 1499827319559);
    }

    #[test]
    fn test_deserialize_listen_key() {
        let json =
            r#"{"listenKey": "pqia91ma19a5s61cv6a81va65sdf19v8a65a1a5s61cv6a81va65sdf19v8a65a1"}"#;
        let response: ListenKeyResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            response.listen_key,
            "pqia91ma19a5s61cv6a81va65sdf19v8a65a1a5s61cv6a81va65sdf19v8a65a1"
        );
    }

    #[test]
    fn test_deserialize_new_order_response() {
        let json = r#"{
            "symbol": "BTCUSDT",
            "orderId": 28,
            "orderListId": -1,
            "clientOrderId": "6gCrw2kRUAF9CvJDGP16IP",
            "transactTime": 1507725176595,
            "price": "0.00000000",
            "origQty": "10.00000000",
            "executedQty": "10.00000000",
            "cummulativeQuoteQty": "10.00000000",
            "status": "FILLED",
            "timeInForce": "GTC",
            "type": "MARKET",
            "side": "SELL",
            "fills": [
                {
                    "price": "4000.00000000",
                    "qty": "1.00000000",
                    "commission": "4.00000000",
                    "commissionAsset": "USDT",
                    "tradeId": 123
                }
            ]
        }"#;

        let response: NewOrderResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.symbol, "BTCUSDT");
        assert_eq!(response.order_id, 28);
        assert_eq!(response.status, "FILLED");
        assert_eq!(response.fills.len(), 1);
    }
}
