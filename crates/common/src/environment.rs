//! Binance environment configuration.
//!
//! Supports production and testnet environments with appropriate URLs.

use std::fmt;
use std::str::FromStr;

/// Binance environment (production or testnet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinanceEnvironment {
    /// Production environment (real money).
    #[default]
    Production,
    /// Testnet environment (fake money for testing).
    Testnet,
}

impl BinanceEnvironment {
    /// REST API base URL.
    pub fn rest_base_url(&self) -> &'static str {
        match self {
            Self::Production => "https://api.binance.com",
            Self::Testnet => "https://testnet.binance.vision",
        }
    }

    /// WebSocket base URL for market data streams.
    pub fn ws_base_url(&self) -> &'static str {
        match self {
            Self::Production => "wss://stream.binance.com:9443",
            Self::Testnet => "wss://testnet.binance.vision",
        }
    }

    /// Returns true if this is the production environment.
    pub fn is_production(&self) -> bool {
        matches!(self, Self::Production)
    }

    /// Returns true if this is the testnet environment.
    pub fn is_testnet(&self) -> bool {
        matches!(self, Self::Testnet)
    }

    /// Load environment from `BINANCE_ENVIRONMENT` env var.
    ///
    /// Returns `Production` if not set or invalid.
    pub fn from_env() -> Self {
        std::env::var("BINANCE_ENVIRONMENT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default()
    }
}

impl fmt::Display for BinanceEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Production => write!(f, "production"),
            Self::Testnet => write!(f, "testnet"),
        }
    }
}

impl FromStr for BinanceEnvironment {
    type Err = ParseEnvironmentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "production" | "prod" | "mainnet" | "main" => Ok(Self::Production),
            "testnet" | "test" | "sandbox" => Ok(Self::Testnet),
            _ => Err(ParseEnvironmentError(s.to_string())),
        }
    }
}

/// Error parsing environment string.
#[derive(Debug, Clone)]
pub struct ParseEnvironmentError(String);

impl fmt::Display for ParseEnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid environment '{}', expected 'production' or 'testnet'",
            self.0
        )
    }
}

impl std::error::Error for ParseEnvironmentError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_production_urls() {
        let env = BinanceEnvironment::Production;
        assert_eq!(env.rest_base_url(), "https://api.binance.com");
        assert_eq!(env.ws_base_url(), "wss://stream.binance.com:9443");
        assert!(env.is_production());
        assert!(!env.is_testnet());
    }

    #[test]
    fn test_testnet_urls() {
        let env = BinanceEnvironment::Testnet;
        assert_eq!(env.rest_base_url(), "https://testnet.binance.vision");
        assert_eq!(env.ws_base_url(), "wss://testnet.binance.vision");
        assert!(!env.is_production());
        assert!(env.is_testnet());
    }

    #[test]
    fn test_parse_production() {
        assert_eq!(
            "production".parse::<BinanceEnvironment>().unwrap(),
            BinanceEnvironment::Production
        );
        assert_eq!(
            "prod".parse::<BinanceEnvironment>().unwrap(),
            BinanceEnvironment::Production
        );
        assert_eq!(
            "MAINNET".parse::<BinanceEnvironment>().unwrap(),
            BinanceEnvironment::Production
        );
    }

    #[test]
    fn test_parse_testnet() {
        assert_eq!(
            "testnet".parse::<BinanceEnvironment>().unwrap(),
            BinanceEnvironment::Testnet
        );
        assert_eq!(
            "test".parse::<BinanceEnvironment>().unwrap(),
            BinanceEnvironment::Testnet
        );
        assert_eq!(
            "SANDBOX".parse::<BinanceEnvironment>().unwrap(),
            BinanceEnvironment::Testnet
        );
    }

    #[test]
    fn test_parse_invalid() {
        assert!("invalid".parse::<BinanceEnvironment>().is_err());
    }

    #[test]
    fn test_default() {
        assert_eq!(
            BinanceEnvironment::default(),
            BinanceEnvironment::Production
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(BinanceEnvironment::Production.to_string(), "production");
        assert_eq!(BinanceEnvironment::Testnet.to_string(), "testnet");
    }
}
