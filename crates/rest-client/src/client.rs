//! Generic REST client wrapper around reqwest.

use crate::error::RestError;
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use std::time::Duration;

/// Default request timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Generic REST client for making HTTP requests.
pub struct RestClient {
    client: Client,
    base_url: String,
}

impl RestClient {
    /// Create a new REST client with the given base URL.
    ///
    /// # Arguments
    /// * `base_url` - Base URL for all requests (e.g., "https://api.binance.com")
    /// * `timeout` - Request timeout duration
    ///
    /// # Errors
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, RestError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| RestError::RequestBuild(e.to_string()))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Create a new REST client with default timeout.
    pub fn with_default_timeout(base_url: &str) -> Result<Self, RestError> {
        Self::new(base_url, DEFAULT_TIMEOUT)
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Make a GET request.
    ///
    /// # Arguments
    /// * `path` - Request path (e.g., "/api/v3/time")
    /// * `query` - Optional query string (without leading '?')
    /// * `headers` - Optional additional headers
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<T, RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "GET request");

        let mut request = self.client.get(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_response(response).await
    }

    /// Make a POST request.
    ///
    /// # Arguments
    /// * `path` - Request path
    /// * `query` - Optional query string (used as body for form-encoded requests)
    /// * `headers` - Optional additional headers
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<T, RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "POST request");

        let mut request = self.client.post(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_response(response).await
    }

    /// Make a POST request that returns an empty response.
    pub async fn post_empty(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<(), RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "POST request (empty response)");

        let mut request = self.client.post(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_empty_response(response).await
    }

    /// Make a PUT request.
    pub async fn put<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<T, RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "PUT request");

        let mut request = self.client.put(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_response(response).await
    }

    /// Make a PUT request that returns an empty response.
    pub async fn put_empty(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<(), RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "PUT request (empty response)");

        let mut request = self.client.put(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_empty_response(response).await
    }

    /// Make a DELETE request.
    pub async fn delete<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<T, RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "DELETE request");

        let mut request = self.client.delete(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_response(response).await
    }

    /// Make a DELETE request that returns an empty response.
    pub async fn delete_empty(
        &self,
        path: &str,
        query: Option<&str>,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<(), RestError> {
        let url = self.build_url(path, query);
        tracing::debug!(url = %url, "DELETE request (empty response)");

        let mut request = self.client.delete(&url);

        if let Some(hdrs) = headers {
            for (key, value) in hdrs {
                request = request.header(*key, *value);
            }
        }

        let response = request.send().await?;
        self.handle_empty_response(response).await
    }

    /// Build a full URL from path and optional query string.
    fn build_url(&self, path: &str, query: Option<&str>) -> String {
        match query {
            Some(q) if !q.is_empty() => format!("{}{}?{}", self.base_url, path, q),
            _ => format!("{}{}", self.base_url, path),
        }
    }

    /// Handle HTTP response and deserialize JSON body.
    async fn handle_response<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, RestError> {
        let status = response.status();

        if status.is_success() {
            let body = response.text().await?;
            serde_json::from_str(&body).map_err(|e| {
                tracing::warn!(body = %body, error = %e, "Failed to parse response");
                RestError::Parse(e.to_string())
            })
        } else {
            let body = response.text().await.unwrap_or_default();

            // Check for rate limiting
            if status.as_u16() == 429 {
                // Try to parse retry-after from response
                return Err(RestError::RateLimited {
                    retry_after_ms: 60_000, // Default to 60 seconds
                });
            }

            Err(RestError::HttpError {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    /// Handle HTTP response for endpoints that return empty body.
    async fn handle_empty_response(&self, response: Response) -> Result<(), RestError> {
        let status = response.status();

        if status.is_success() {
            Ok(())
        } else {
            let body = response.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                return Err(RestError::RateLimited {
                    retry_after_ms: 60_000,
                });
            }

            Err(RestError::HttpError {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url_no_query() {
        let client = RestClient::with_default_timeout("https://api.example.com").unwrap();
        assert_eq!(
            client.build_url("/api/v1/time", None),
            "https://api.example.com/api/v1/time"
        );
    }

    #[test]
    fn test_build_url_with_query() {
        let client = RestClient::with_default_timeout("https://api.example.com").unwrap();
        assert_eq!(
            client.build_url("/api/v1/order", Some("symbol=BTCUSDT&side=BUY")),
            "https://api.example.com/api/v1/order?symbol=BTCUSDT&side=BUY"
        );
    }

    #[test]
    fn test_build_url_strips_trailing_slash() {
        let client = RestClient::with_default_timeout("https://api.example.com/").unwrap();
        assert_eq!(
            client.build_url("/api/v1/time", None),
            "https://api.example.com/api/v1/time"
        );
    }

    #[test]
    fn test_build_url_empty_query() {
        let client = RestClient::with_default_timeout("https://api.example.com").unwrap();
        assert_eq!(
            client.build_url("/api/v1/time", Some("")),
            "https://api.example.com/api/v1/time"
        );
    }
}
