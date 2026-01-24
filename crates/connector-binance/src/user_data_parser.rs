//! User data stream message parser.
//!
//! Parses messages from Binance user data WebSocket stream including:
//! - executionReport - Order status updates
//! - outboundAccountPosition - Account balance updates

use execution_core::{ExecutionReport, OrderSide, OrderStatus, OrderType, TimeInForce};
use rust_decimal::Decimal;
use serde::Deserialize;

/// Raw execution report from Binance WebSocket.
#[derive(Debug, Deserialize)]
pub struct BinanceExecutionReportRaw {
    /// Event type (should be "executionReport")
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time
    #[serde(rename = "E")]
    pub event_time: i64,
    /// Symbol
    #[serde(rename = "s")]
    pub symbol: String,
    /// Client order ID
    #[serde(rename = "c")]
    pub client_order_id: String,
    /// Side (BUY/SELL)
    #[serde(rename = "S")]
    pub side: String,
    /// Order type
    #[serde(rename = "o")]
    pub order_type: String,
    /// Time in force
    #[serde(rename = "f")]
    pub time_in_force: String,
    /// Order quantity
    #[serde(rename = "q")]
    pub quantity: Decimal,
    /// Order price
    #[serde(rename = "p")]
    pub price: Decimal,
    /// Current order status
    #[serde(rename = "X")]
    pub order_status: String,
    /// Order ID
    #[serde(rename = "i")]
    pub order_id: u64,
    /// Last executed quantity
    #[serde(rename = "l")]
    pub last_executed_qty: Decimal,
    /// Cumulative filled quantity
    #[serde(rename = "z")]
    pub cumulative_filled_qty: Decimal,
    /// Last executed price
    #[serde(rename = "L")]
    pub last_executed_price: Decimal,
    /// Commission amount
    #[serde(rename = "n")]
    pub commission: Decimal,
    /// Commission asset
    #[serde(rename = "N")]
    pub commission_asset: Option<String>,
    /// Trade time
    #[serde(rename = "T")]
    pub trade_time: i64,
    /// Trade ID (-1 if no trade)
    #[serde(rename = "t")]
    pub trade_id: i64,
    /// Is this the maker side?
    #[serde(rename = "m")]
    pub is_maker: bool,
}

/// Raw account update from Binance WebSocket.
#[derive(Debug, Deserialize)]
pub struct BinanceAccountUpdateRaw {
    /// Event type (should be "outboundAccountPosition")
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time
    #[serde(rename = "E")]
    pub event_time: i64,
    /// Time of last account update
    #[serde(rename = "u")]
    pub last_update_time: i64,
    /// Balances
    #[serde(rename = "B")]
    pub balances: Vec<BinanceBalanceRaw>,
}

/// Raw balance from Binance WebSocket.
#[derive(Debug, Deserialize)]
pub struct BinanceBalanceRaw {
    /// Asset
    #[serde(rename = "a")]
    pub asset: String,
    /// Free balance
    #[serde(rename = "f")]
    pub free: Decimal,
    /// Locked balance
    #[serde(rename = "l")]
    pub locked: Decimal,
}

/// Parsed user data message.
#[derive(Debug)]
pub enum UserDataMessage {
    /// Order execution report
    ExecutionReport(ExecutionReport),
    /// Account balance update
    AccountUpdate(AccountUpdate),
    /// Unknown or unhandled message type
    Unknown,
}

/// Account balance update.
#[derive(Debug, Clone)]
pub struct AccountUpdate {
    /// Event timestamp
    pub event_time_ms: i64,
    /// Updated balances
    pub balances: Vec<BalanceUpdate>,
}

/// Single balance update.
#[derive(Debug, Clone)]
pub struct BalanceUpdate {
    /// Asset symbol (e.g., "BTC", "USDT")
    pub asset: String,
    /// Free (available) balance
    pub free: Decimal,
    /// Locked balance (in open orders)
    pub locked: Decimal,
}

/// Parse a user data stream message.
pub fn parse_user_data_message(text: &str) -> Result<UserDataMessage, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(text)?;

    match value.get("e").and_then(|v| v.as_str()) {
        Some("executionReport") => {
            let raw: BinanceExecutionReportRaw = serde_json::from_value(value)?;
            Ok(UserDataMessage::ExecutionReport(raw.into()))
        }
        Some("outboundAccountPosition") => {
            let raw: BinanceAccountUpdateRaw = serde_json::from_value(value)?;
            Ok(UserDataMessage::AccountUpdate(raw.into()))
        }
        _ => Ok(UserDataMessage::Unknown),
    }
}

impl From<BinanceExecutionReportRaw> for ExecutionReport {
    fn from(raw: BinanceExecutionReportRaw) -> Self {
        ExecutionReport {
            event_time_ms: raw.event_time,
            symbol: raw.symbol,
            client_order_id: raw.client_order_id,
            side: parse_order_side(&raw.side),
            order_type: parse_order_type(&raw.order_type),
            time_in_force: parse_time_in_force(&raw.time_in_force),
            quantity: raw.quantity,
            price: raw.price,
            order_status: parse_order_status(&raw.order_status),
            order_id: raw.order_id,
            last_executed_qty: raw.last_executed_qty,
            cumulative_filled_qty: raw.cumulative_filled_qty,
            last_executed_price: raw.last_executed_price,
            commission: raw.commission,
            commission_asset: raw.commission_asset.unwrap_or_default(),
            trade_time_ms: raw.trade_time,
            trade_id: raw.trade_id,
            is_maker: raw.is_maker,
        }
    }
}

impl From<BinanceAccountUpdateRaw> for AccountUpdate {
    fn from(raw: BinanceAccountUpdateRaw) -> Self {
        AccountUpdate {
            event_time_ms: raw.event_time,
            balances: raw.balances.into_iter().map(|b| b.into()).collect(),
        }
    }
}

impl From<BinanceBalanceRaw> for BalanceUpdate {
    fn from(raw: BinanceBalanceRaw) -> Self {
        BalanceUpdate {
            asset: raw.asset,
            free: raw.free,
            locked: raw.locked,
        }
    }
}

fn parse_order_side(s: &str) -> OrderSide {
    OrderSide::from_binance_str(s).unwrap_or(OrderSide::Buy)
}

fn parse_order_type(s: &str) -> OrderType {
    OrderType::from_binance_str(s).unwrap_or(OrderType::Limit)
}

fn parse_time_in_force(s: &str) -> TimeInForce {
    TimeInForce::from_binance_str(s).unwrap_or(TimeInForce::GTC)
}

fn parse_order_status(s: &str) -> OrderStatus {
    OrderStatus::from_binance_str(s).unwrap_or(OrderStatus::New)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_execution_report() {
        let json = r#"{
            "e": "executionReport",
            "E": 1499405658658,
            "s": "ETHBTC",
            "c": "mUvoqJxFIILMdfAW5iGSOW",
            "S": "BUY",
            "o": "LIMIT",
            "f": "GTC",
            "q": "1.00000000",
            "p": "0.10264410",
            "X": "NEW",
            "i": 4293153,
            "l": "0.00000000",
            "z": "0.00000000",
            "L": "0.00000000",
            "n": "0",
            "N": null,
            "T": 1499405658657,
            "t": -1,
            "m": false
        }"#;

        let result = parse_user_data_message(json).unwrap();
        match result {
            UserDataMessage::ExecutionReport(report) => {
                assert_eq!(report.symbol, "ETHBTC");
                assert_eq!(report.client_order_id, "mUvoqJxFIILMdfAW5iGSOW");
                assert_eq!(report.side, OrderSide::Buy);
                assert_eq!(report.order_status, OrderStatus::New);
            }
            _ => panic!("Expected ExecutionReport"),
        }
    }

    #[test]
    fn test_parse_account_update() {
        let json = r#"{
            "e": "outboundAccountPosition",
            "E": 1564034571105,
            "u": 1564034571073,
            "B": [
                {
                    "a": "ETH",
                    "f": "10000.000000",
                    "l": "0.000000"
                },
                {
                    "a": "BTC",
                    "f": "1.000000",
                    "l": "0.500000"
                }
            ]
        }"#;

        let result = parse_user_data_message(json).unwrap();
        match result {
            UserDataMessage::AccountUpdate(update) => {
                assert_eq!(update.balances.len(), 2);
                assert_eq!(update.balances[0].asset, "ETH");
                assert_eq!(update.balances[1].asset, "BTC");
            }
            _ => panic!("Expected AccountUpdate"),
        }
    }

    #[test]
    fn test_parse_unknown_message() {
        let json = r#"{"e": "someOtherEvent", "data": "something"}"#;

        let result = parse_user_data_message(json).unwrap();
        assert!(matches!(result, UserDataMessage::Unknown));
    }
}
