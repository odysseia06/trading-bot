//! Binance REST API client.

use crate::error::BinanceRestError;
use crate::responses::{
    DepthSnapshotResponse, ListenKeyResponse, NewOrderResponse, OrderQueryResponse,
    ServerTimeResponse,
};
use auth::{ApiCredentials, RequestSigner};
use common::BinanceEnvironment;
use execution_core::{OrderSide, OrderType, TimeInForce};
use rest_client::RestClient;
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

/// Request timeout for Binance API calls.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Binance REST API client with authentication support.
pub struct BinanceRestClient {
    client: RestClient,
    credentials: ApiCredentials,
    environment: BinanceEnvironment,
    /// Time offset between local clock and Binance server (local - server).
    time_offset_ms: AtomicI64,
}

impl BinanceRestClient {
    /// Create a new Binance REST client for production.
    ///
    /// # Arguments
    /// * `credentials` - API credentials for authenticated requests
    ///
    /// # Errors
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(credentials: ApiCredentials) -> Result<Self, BinanceRestError> {
        Self::with_environment(credentials, BinanceEnvironment::Production)
    }

    /// Create a new Binance REST client for a specific environment.
    ///
    /// # Arguments
    /// * `credentials` - API credentials for authenticated requests
    /// * `environment` - Production or Testnet
    ///
    /// # Errors
    /// Returns an error if the HTTP client cannot be built.
    pub fn with_environment(
        credentials: ApiCredentials,
        environment: BinanceEnvironment,
    ) -> Result<Self, BinanceRestError> {
        let client = RestClient::new(environment.rest_base_url(), REQUEST_TIMEOUT)?;

        Ok(Self {
            client,
            credentials,
            environment,
            time_offset_ms: AtomicI64::new(0),
        })
    }

    /// Get the environment this client is connected to.
    pub fn environment(&self) -> BinanceEnvironment {
        self.environment
    }

    /// Get the API key (for logging/debugging).
    pub fn api_key(&self) -> &str {
        self.credentials.api_key()
    }

    /// Get the current server timestamp adjusted for time offset.
    ///
    /// This returns the estimated current Binance server time based on
    /// the local clock and the calculated time offset.
    pub fn server_timestamp_ms(&self) -> i64 {
        let local_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        local_time - self.time_offset_ms.load(Ordering::Relaxed)
    }

    // ========================================================================
    // Time Synchronization
    // ========================================================================

    /// Synchronize with Binance server time.
    ///
    /// This calculates the offset between the local clock and the server clock.
    /// Should be called on startup and periodically if timestamps are being rejected.
    pub async fn sync_time(&self) -> Result<(), BinanceRestError> {
        let before = std::time::Instant::now();
        let response: ServerTimeResponse = self.client.get("/api/v3/time", None, None).await?;
        let rtt = before.elapsed().as_millis() as i64;

        let local_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Estimate server time at midpoint of request
        let estimated_server_time = response.server_time + (rtt / 2);
        let offset = local_time - estimated_server_time;

        self.time_offset_ms.store(offset, Ordering::Relaxed);

        tracing::info!(
            server_time = response.server_time,
            local_time = local_time,
            offset_ms = offset,
            rtt_ms = rtt,
            "Time synchronized with Binance server"
        );

        Ok(())
    }

    // ========================================================================
    // Market Data
    // ========================================================================

    /// Get order book depth snapshot.
    ///
    /// GET /api/v3/depth
    ///
    /// # Parameters
    /// - `symbol`: Trading pair (e.g., "BTCUSDT")
    /// - `limit`: Number of levels (5, 10, 20, 50, 100, 500, 1000, 5000). Default: 100
    ///
    /// This is used to initialize the order book before applying depth updates
    /// from the WebSocket stream.
    pub async fn get_depth_snapshot(
        &self,
        symbol: &str,
        limit: u32,
    ) -> Result<DepthSnapshotResponse, BinanceRestError> {
        let query = format!("symbol={}&limit={}", symbol, limit);

        tracing::debug!(symbol = %symbol, limit = limit, "Fetching depth snapshot");

        let response: DepthSnapshotResponse =
            self.client.get("/api/v3/depth", Some(&query), None).await?;

        tracing::debug!(
            symbol = %symbol,
            last_update_id = response.last_update_id,
            bid_levels = response.bids.len(),
            ask_levels = response.asks.len(),
            "Depth snapshot received"
        );

        Ok(response)
    }

    // ========================================================================
    // Listen Key Management
    // ========================================================================

    /// Create a new listen key for user data stream.
    ///
    /// POST /api/v3/userDataStream
    ///
    /// The listen key is valid for 60 minutes and must be refreshed periodically.
    pub async fn create_listen_key(&self) -> Result<String, BinanceRestError> {
        let headers = [("X-MBX-APIKEY", self.credentials.api_key())];

        let response: ListenKeyResponse = self
            .client
            .post("/api/v3/userDataStream", None, Some(&headers))
            .await?;

        tracing::info!(
            listen_key = %response.listen_key,
            "Created listen key"
        );

        Ok(response.listen_key)
    }

    /// Refresh (keep-alive) an existing listen key.
    ///
    /// PUT /api/v3/userDataStream
    ///
    /// Should be called every 30 minutes to keep the listen key active.
    /// The key expires after 60 minutes of inactivity.
    pub async fn keepalive_listen_key(&self, listen_key: &str) -> Result<(), BinanceRestError> {
        let headers = [("X-MBX-APIKEY", self.credentials.api_key())];
        let query = format!("listenKey={}", listen_key);

        self.client
            .put_empty("/api/v3/userDataStream", Some(&query), Some(&headers))
            .await?;

        tracing::debug!("Listen key refreshed");
        Ok(())
    }

    /// Close a listen key.
    ///
    /// DELETE /api/v3/userDataStream
    ///
    /// Should be called when shutting down to clean up resources.
    pub async fn close_listen_key(&self, listen_key: &str) -> Result<(), BinanceRestError> {
        let headers = [("X-MBX-APIKEY", self.credentials.api_key())];
        let query = format!("listenKey={}", listen_key);

        self.client
            .delete_empty("/api/v3/userDataStream", Some(&query), Some(&headers))
            .await?;

        tracing::info!("Listen key closed");
        Ok(())
    }

    // ========================================================================
    // Order Management
    // ========================================================================

    /// Place a new order.
    ///
    /// POST /api/v3/order
    ///
    /// # Parameters
    /// - `symbol`: Trading pair (e.g., "BTCUSDT")
    /// - `side`: Buy or Sell
    /// - `order_type`: Market, Limit, StopLoss, StopLossLimit, TakeProfit, TakeProfitLimit, LimitMaker
    /// - `quantity`: Order quantity
    /// - `price`: Limit price (required for Limit, StopLossLimit, TakeProfitLimit, LimitMaker)
    /// - `stop_price`: Trigger price (required for StopLoss, StopLossLimit, TakeProfit, TakeProfitLimit)
    /// - `time_in_force`: GTC, IOC, FOK (required for Limit orders)
    /// - `client_order_id`: Custom order ID for tracking
    pub async fn place_order(
        &self,
        symbol: &str,
        side: OrderSide,
        order_type: OrderType,
        quantity: Decimal,
        price: Option<Decimal>,
        stop_price: Option<Decimal>,
        time_in_force: Option<TimeInForce>,
        client_order_id: &str,
    ) -> Result<NewOrderResponse, BinanceRestError> {
        let mut params: Vec<(&str, String)> = vec![
            ("symbol", symbol.to_string()),
            ("side", side.as_binance_str().to_string()),
            ("type", order_type.as_binance_str().to_string()),
            ("quantity", quantity.to_string()),
            ("newClientOrderId", client_order_id.to_string()),
            ("newOrderRespType", "FULL".to_string()),
        ];

        if let Some(p) = price {
            params.push(("price", p.to_string()));
        }

        if let Some(sp) = stop_price {
            params.push(("stopPrice", sp.to_string()));
        }

        if let Some(tif) = time_in_force {
            params.push(("timeInForce", tif.as_binance_str().to_string()));
        }

        // Convert to slice of references for signer
        let param_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let signer = RequestSigner::new(&self.credentials);
        let timestamp = self.server_timestamp_ms();
        let signed_query = signer.sign_params_ordered(&param_refs, timestamp);

        let headers = [("X-MBX-APIKEY", self.credentials.api_key())];

        tracing::info!(
            symbol = %symbol,
            side = ?side,
            order_type = ?order_type,
            quantity = %quantity,
            client_order_id = %client_order_id,
            "Placing order"
        );

        let response: NewOrderResponse = self
            .client
            .post("/api/v3/order", Some(&signed_query), Some(&headers))
            .await?;

        tracing::info!(
            order_id = response.order_id,
            status = %response.status,
            "Order placed"
        );

        Ok(response)
    }

    /// Query an order by exchange order ID.
    ///
    /// GET /api/v3/order
    pub async fn query_order_by_id(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> Result<OrderQueryResponse, BinanceRestError> {
        let params = [("symbol", symbol), ("orderId", &order_id.to_string())];

        self.query_order_internal(&params).await
    }

    /// Query an order by client order ID.
    ///
    /// GET /api/v3/order
    pub async fn query_order_by_client_id(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<OrderQueryResponse, BinanceRestError> {
        let params = [("symbol", symbol), ("origClientOrderId", client_order_id)];

        self.query_order_internal(&params).await
    }

    /// Internal order query implementation.
    async fn query_order_internal(
        &self,
        params: &[(&str, &str)],
    ) -> Result<OrderQueryResponse, BinanceRestError> {
        let signer = RequestSigner::new(&self.credentials);
        let timestamp = self.server_timestamp_ms();
        let signed_query = signer.sign_params(params, timestamp);

        let headers = [("X-MBX-APIKEY", self.credentials.api_key())];

        let response: OrderQueryResponse = self
            .client
            .get("/api/v3/order", Some(&signed_query), Some(&headers))
            .await?;

        Ok(response)
    }

    /// Cancel an order by exchange order ID.
    ///
    /// DELETE /api/v3/order
    pub async fn cancel_order_by_id(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> Result<OrderQueryResponse, BinanceRestError> {
        let params = [("symbol", symbol), ("orderId", &order_id.to_string())];

        self.cancel_order_internal(&params).await
    }

    /// Cancel an order by client order ID.
    ///
    /// DELETE /api/v3/order
    pub async fn cancel_order_by_client_id(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<OrderQueryResponse, BinanceRestError> {
        let params = [("symbol", symbol), ("origClientOrderId", client_order_id)];

        self.cancel_order_internal(&params).await
    }

    /// Internal order cancellation implementation.
    async fn cancel_order_internal(
        &self,
        params: &[(&str, &str)],
    ) -> Result<OrderQueryResponse, BinanceRestError> {
        let signer = RequestSigner::new(&self.credentials);
        let timestamp = self.server_timestamp_ms();
        let signed_query = signer.sign_params(params, timestamp);

        let headers = [("X-MBX-APIKEY", self.credentials.api_key())];

        tracing::info!(
            params = ?params,
            "Canceling order"
        );

        let response: OrderQueryResponse = self
            .client
            .delete("/api/v3/order", Some(&signed_query), Some(&headers))
            .await?;

        tracing::info!(
            order_id = response.order_id,
            status = %response.status,
            "Order canceled"
        );

        Ok(response)
    }
}

impl std::fmt::Debug for BinanceRestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinanceRestClient")
            .field("environment", &self.environment)
            .field("base_url", &self.environment.rest_base_url())
            .field("api_key", &self.credentials.api_key())
            .field(
                "time_offset_ms",
                &self.time_offset_ms.load(Ordering::Relaxed),
            )
            .finish()
    }
}
