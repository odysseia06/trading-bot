//! HMAC-SHA256 request signing for Binance API.

use crate::credentials::ApiCredentials;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Request signer for authenticated Binance API calls.
pub struct RequestSigner<'a> {
    credentials: &'a ApiCredentials,
}

impl<'a> RequestSigner<'a> {
    /// Create a new request signer with the given credentials.
    pub fn new(credentials: &'a ApiCredentials) -> Self {
        Self { credentials }
    }

    /// Sign a message and return the hex-encoded signature.
    ///
    /// This computes HMAC-SHA256 of the message using the secret key
    /// and returns the result as a lowercase hex string.
    pub fn sign(&self, message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.credentials.expose_secret().as_bytes())
            .expect("HMAC can take key of any size");

        mac.update(message.as_bytes());
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Build a signed query string from parameters.
    ///
    /// This method:
    /// 1. Adds the timestamp to the parameters
    /// 2. Sorts all parameters alphabetically by key
    /// 3. Builds a query string
    /// 4. Signs the query string
    /// 5. Appends the signature
    ///
    /// # Arguments
    /// * `params` - Key-value pairs to include in the query string
    /// * `timestamp_ms` - Current timestamp in milliseconds
    ///
    /// # Returns
    /// A complete query string with signature appended
    pub fn sign_params(&self, params: &[(&str, &str)], timestamp_ms: i64) -> String {
        let mut all_params: Vec<(&str, String)> =
            params.iter().map(|(k, v)| (*k, v.to_string())).collect();

        // Add timestamp
        all_params.push(("timestamp", timestamp_ms.to_string()));

        // Sort alphabetically by key (Binance requirement for some endpoints)
        all_params.sort_by(|a, b| a.0.cmp(b.0));

        // Build query string
        let query_string = all_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        // Sign and append
        let signature = self.sign(&query_string);
        format!("{}&signature={}", query_string, signature)
    }

    /// Build a signed query string without sorting (preserves parameter order).
    ///
    /// Some Binance endpoints expect parameters in a specific order.
    /// This method adds timestamp at the end, then signs.
    pub fn sign_params_ordered(&self, params: &[(&str, &str)], timestamp_ms: i64) -> String {
        let mut query_parts: Vec<String> =
            params.iter().map(|(k, v)| format!("{}={}", k, v)).collect();

        // Add timestamp at the end
        query_parts.push(format!("timestamp={}", timestamp_ms));

        let query_string = query_parts.join("&");
        let signature = self.sign(&query_string);
        format!("{}&signature={}", query_string, signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_known_vector() {
        // Test vector from Binance API documentation
        // https://binance-docs.github.io/apidocs/spot/en/#signed-trade-and-user_data-endpoint-security
        let creds = ApiCredentials::new(
            "vmPUZE6mv9SD5VNHk4HlWFsOr6aKE2zvsw0MuIgwCIPy6utIco14y7Ju91duEh8A".into(),
            "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j".into(),
        );

        let signer = RequestSigner::new(&creds);

        // From Binance docs example
        let query = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        let signature = signer.sign(query);

        assert_eq!(
            signature,
            "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
        );
    }

    #[test]
    fn test_sign_params_includes_timestamp() {
        let creds = ApiCredentials::new("key".into(), "secret".into());
        let signer = RequestSigner::new(&creds);

        let params = [("symbol", "BTCUSDT")];
        let result = signer.sign_params(&params, 1000);

        assert!(result.contains("timestamp=1000"));
        assert!(result.contains("signature="));
    }

    #[test]
    fn test_sign_params_sorted() {
        let creds = ApiCredentials::new("key".into(), "secret".into());
        let signer = RequestSigner::new(&creds);

        let params = [("zebra", "1"), ("alpha", "2"), ("middle", "3")];
        let result = signer.sign_params(&params, 1000);

        // Should be sorted: alpha, middle, timestamp, zebra
        let signature_pos = result.find("&signature=").unwrap();
        let query_part = &result[..signature_pos];

        assert!(query_part.starts_with("alpha=2"));
        assert!(query_part.contains("&middle=3"));
        assert!(query_part.contains("&timestamp=1000"));
        assert!(query_part.contains("&zebra=1"));
    }

    #[test]
    fn test_sign_params_ordered_preserves_order() {
        let creds = ApiCredentials::new("key".into(), "secret".into());
        let signer = RequestSigner::new(&creds);

        let params = [("zebra", "1"), ("alpha", "2")];
        let result = signer.sign_params_ordered(&params, 1000);

        // Order should be preserved: zebra, alpha, timestamp
        let signature_pos = result.find("&signature=").unwrap();
        let query_part = &result[..signature_pos];

        assert!(query_part.starts_with("zebra=1&alpha=2&timestamp=1000"));
    }

    #[test]
    fn test_sign_empty_message() {
        let creds = ApiCredentials::new("key".into(), "secret".into());
        let signer = RequestSigner::new(&creds);

        // Should not panic on empty message
        let signature = signer.sign("");
        assert!(!signature.is_empty());
    }
}
