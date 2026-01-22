//! Kalshi REST + WebSocket client

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Client as HttpClient;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

use super::auth::KalshiAuth;
use super::models::*;
use super::websocket::KalshiWebSocket;
use crate::config::KalshiConfig;
use crate::error::{Error, Result};
use crate::constants::KALSHI_REST_URL;
use crate::constants::KALSHI_WS_URL;

/// Kalshi client for REST and WebSocket operations
pub struct KalshiClient {
    config: KalshiConfig,
    http: HttpClient,
    auth: Arc<KalshiAuth>,
    ws: Option<KalshiWebSocket>,
    /// Cache of known markets
    markets_cache: Arc<RwLock<HashMap<String, KalshiMarket>>>,
    /// Currently tracked markets
    tracked_markets: Arc<RwLock<HashMap<String, KalshiMarket>>>,
}

impl KalshiClient {
    /// Create a new Kalshi client
    pub fn new(config: KalshiConfig) -> Result<Self> {
        let auth = KalshiAuth::from_file(&config.api_key_id, &config.private_key_path)?;

        Ok(Self {
            config,
            http: HttpClient::new(),
            auth: Arc::new(auth),
            ws: None,
            markets_cache: Arc::new(RwLock::new(HashMap::new())),
            tracked_markets: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Get HTTP headers for authenticated requests
    fn auth_headers(&self, method: &str, path: &str) -> Result<HashMap<String, String>> {
        let headers = self.auth.generate_headers(method, path)?;
        let mut map = HashMap::new();
        map.insert("KALSHI-ACCESS-KEY".to_string(), headers.api_key);
        map.insert("KALSHI-ACCESS-TIMESTAMP".to_string(), headers.timestamp);
        map.insert("KALSHI-ACCESS-SIGNATURE".to_string(), headers.signature);
        Ok(map)
    }

    // ========================================================================
    // REST API Methods
    // ========================================================================

    /// Fetch markets from REST API
    pub async fn fetch_markets(
        &self,
        status: Option<&str>,
        series_ticker: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<MarketsResponse> {
        let path = "/trade-api/v2/markets";
        let mut url = format!("{}{}", KALSHI_REST_URL, path);

        let mut params = vec![];
        if let Some(s) = status {
            params.push(format!("status={}", s));
        }
        if let Some(s) = series_ticker {
            params.push(format!("series_ticker={}", s));
        }
        if let Some(c) = cursor {
            params.push(format!("cursor={}", c));
        }
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }

        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        let auth = self.auth_headers("GET", path)?;

        let resp = self
            .http
            .get(&url)
            .header("KALSHI-ACCESS-KEY", &auth["KALSHI-ACCESS-KEY"])
            .header("KALSHI-ACCESS-TIMESTAMP", &auth["KALSHI-ACCESS-TIMESTAMP"])
            .header("KALSHI-ACCESS-SIGNATURE", &auth["KALSHI-ACCESS-SIGNATURE"])
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("HTTP {}: {}", status, body)));
        }

        let data: MarketsResponse = resp
            .json()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        // Update cache
        {
            let mut cache = self.markets_cache.write().await;
            for market in &data.markets {
                cache.insert(market.ticker.clone(), market.clone());
            }
        }

        Ok(data)
    }

    /// Fetch all markets matching a filter (handles pagination)
    pub async fn fetch_all_markets(
        &self,
        status: Option<&str>,
        filter: impl Fn(&KalshiMarket) -> bool,
    ) -> Result<Vec<KalshiMarket>> {
        let mut all_markets = Vec::new();
        let mut cursor = None;

        loop {
            let resp = self
                .fetch_markets(status, None, cursor.as_deref(), Some(100))
                .await?;

            all_markets.extend(resp.markets.into_iter().filter(&filter));

            if resp.cursor.is_none() {
                break;
            }
            cursor = resp.cursor;
        }

        Ok(all_markets)
    }

    /// Fetch markets matching tracked symbols (e.g., ETH15M, BTC15M)
    pub async fn fetch_tracked_markets(&self) -> Result<Vec<KalshiMarket>> {
        let symbols = self.config.tracked_symbols.clone();

        self.fetch_all_markets(Some("active"), |m| {
            let ticker_upper = m.ticker.to_uppercase();
            symbols.iter().any(|s| ticker_upper.contains(s))
        })
        .await
    }

    /// Fetch a specific market by ticker
    pub async fn fetch_market(&self, ticker: &str) -> Result<KalshiMarket> {
        let path = format!("/trade-api/v2/markets/{}", ticker);
        let url = format!("{}{}", KALSHI_REST_URL, path);
        let auth = self.auth_headers("GET", &path)?;

        let resp = self
            .http
            .get(&url)
            .header("KALSHI-ACCESS-KEY", &auth["KALSHI-ACCESS-KEY"])
            .header("KALSHI-ACCESS-TIMESTAMP", &auth["KALSHI-ACCESS-TIMESTAMP"])
            .header("KALSHI-ACCESS-SIGNATURE", &auth["KALSHI-ACCESS-SIGNATURE"])
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(Error::MarketNotFound(ticker.to_string()));
        }

        #[derive(serde::Deserialize)]
        struct MarketResponse {
            market: KalshiMarket,
        }

        let data: MarketResponse = resp
            .json()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        Ok(data.market)
    }

    // ========================================================================
    // WebSocket Methods
    // ========================================================================

    /// Connect WebSocket
    pub async fn connect(&mut self) -> Result<()> {
        let auth = KalshiAuth::from_file(&self.config.api_key_id, &self.config.private_key_path)?;
        let mut ws = KalshiWebSocket::new(KALSHI_WS_URL, auth);
        ws.connect().await?;
        self.ws = Some(ws);
        Ok(())
    }

    /// Disconnect WebSocket
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(ws) = &mut self.ws {
            ws.disconnect().await?;
        }
        self.ws = None;
        Ok(())
    }

    /// Get mutable reference to WebSocket
    pub fn websocket_mut(&mut self) -> Result<&mut KalshiWebSocket> {
        self.ws
            .as_mut()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))
    }

    /// Get reference to WebSocket
    pub fn websocket(&self) -> Result<&KalshiWebSocket> {
        self.ws
            .as_ref()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))
    }

    /// Check if WebSocket is connected
    pub fn is_connected(&self) -> bool {
        self.ws.as_ref().map(|ws| ws.is_connected()).unwrap_or(false)
    }

    // ========================================================================
    // Market Tracking
    // ========================================================================

    /// Start tracking a market
    pub async fn track_market(&self, market: &KalshiMarket) {
        let mut tracked = self.tracked_markets.write().await;
        info!("Tracking market: {} ({})", market.ticker, market.status);
        tracked.insert(market.ticker.clone(), market.clone());
    }

    /// Stop tracking a market
    pub async fn untrack_market(&self, ticker: &str) {
        let mut tracked = self.tracked_markets.write().await;
        tracked.remove(ticker);
        info!("Stopped tracking market: {}", ticker);
    }

    /// Get currently tracked markets
    pub async fn get_tracked_markets(&self) -> Vec<KalshiMarket> {
        let tracked = self.tracked_markets.read().await;
        tracked.values().cloned().collect()
    }

    /// Find next market to track when current one closes
    pub async fn find_next_market(&self, symbol_pattern: &str) -> Result<Option<KalshiMarket>> {
        let markets = self
            .fetch_all_markets(Some("active"), |m| {
                m.ticker
                    .to_uppercase()
                    .contains(&symbol_pattern.to_uppercase())
            })
            .await?;

        // Find the one with earliest close time that's still active
        let next = markets
            .into_iter()
            .filter(|m| m.status.to_lowercase() == "active")
            .min_by_key(|m| m.close_time.clone().unwrap_or_default());

        Ok(next)
    }

    /// Find the next market in a series after the given ticker closes
    pub async fn find_successor_market(&self, closed_ticker: &str) -> Result<Option<KalshiMarket>> {
        let ticker_upper = closed_ticker.to_uppercase();

        let pattern = if ticker_upper.contains("ETH15M") {
            "ETH15M"
        } else if ticker_upper.contains("BTC15M") {
            "BTC15M"
        } else if ticker_upper.contains("SOL15M") {
            "SOL15M"
        } else {
            warn!(
                "Could not determine symbol pattern from ticker: {}",
                closed_ticker
            );
            return Ok(None);
        };

        self.find_next_market(pattern).await
    }

    // ========================================================================
    // High-level operations
    // ========================================================================

    /// Start listening to market events
    /// 
    /// Connects to WebSocket, subscribes to market lifecycle and ticker updates,
    /// and runs the message loop sending events to the provided channel.
    pub async fn start(&mut self, event_tx: mpsc::Sender<KalshiEvent>) -> Result<()> {
        // Connect if not already connected
        if !self.is_connected() {
            self.connect().await?;
        }

        // Subscribe to market lifecycle for all markets
        let ws = self.websocket_mut()?;
        ws.subscribe_market_lifecycle().await?;

        // Fetch initial tracked markets
        let markets = self.fetch_tracked_markets().await?;
        let tickers: Vec<String> = markets.iter().map(|m| m.ticker.clone()).collect();

        if !tickers.is_empty() {
            // Track them
            for market in &markets {
                self.track_market(market).await;
            }

            // Subscribe to their tickers
            let ws = self.websocket_mut()?;
            ws.subscribe_tickers(tickers).await?;
        }

        // Create filter for tracked symbols
        let symbols = self.config.tracked_symbols.clone();
        let filter = Arc::new(move |ticker: &str| {
            let ticker_upper = ticker.to_uppercase();
            symbols.iter().any(|s| ticker_upper.contains(s))
        });

        // Run WebSocket message loop
        let ws = self.websocket_mut()?;
        ws.run(event_tx, Some(filter)).await?;

        Ok(())
    }
}
