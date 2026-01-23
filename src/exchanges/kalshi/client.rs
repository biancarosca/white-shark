use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::info;

use super::api::KalshiApi;
use super::auth::KalshiAuth;
use super::models::*;
use super::websocket::KalshiWebSocket;
use crate::config::KalshiConfig;
use crate::error::{Error, Result};
use crate::constants::KALSHI_WS_URL;

pub struct KalshiClient {
    config: KalshiConfig,
    api: KalshiApi,
    ws: Option<KalshiWebSocket>,
    tracked_markets: Arc<RwLock<HashMap<String, KalshiMarket>>>,
}

impl KalshiClient {
    pub fn new(config: KalshiConfig) -> Result<Self> {
        let auth = KalshiAuth::from_file(&config.api_key_id, &config.private_key_path)?;
        let auth_arc = Arc::new(auth);
        let api = KalshiApi::new(auth_arc.clone());

        Ok(Self {
            config,
            api,
            ws: None,
            tracked_markets: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn connect(&mut self) -> Result<()> {
        let auth = KalshiAuth::from_file(&self.config.api_key_id, &self.config.private_key_path)?;
        let mut ws = KalshiWebSocket::new(KALSHI_WS_URL, auth);
        ws.connect().await?;
        self.ws = Some(ws);
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(ws) = &mut self.ws {
            ws.disconnect().await?;
        }
        self.ws = None;
        Ok(())
    }

    pub fn websocket_mut(&mut self) -> Result<&mut KalshiWebSocket> {
        self.ws
            .as_mut()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))
    }

    pub fn is_connected(&self) -> bool {
        self.ws.as_ref().map(|ws| ws.is_connected()).unwrap_or(false)
    }

    pub async fn track_market(&self, market: &KalshiMarket) {
        let mut tracked = self.tracked_markets.write().await;
        info!("🪄 Tracking market: {} ({:?})", market.ticker, market.status);
        tracked.insert(market.ticker.clone(), market.clone());
    }

    pub async fn start(&mut self, event_tx: mpsc::Sender<KalshiEvent>) -> Result<()> {
        if !self.is_connected() {
            self.connect().await?;
        }
        
        let symbols: Vec<&str> = self.config.tracked_symbols.iter().map(|s| s.as_str()).collect();
        let markets = self.api.get_markets_for_tickers(&symbols).await?;

        let mut tickers = Vec::new();
        for market in &markets {
            self.track_market(market).await;
            tickers.push(market.ticker.clone());
        }

        let ws = self.websocket_mut()?;
        
        ws.subscribe_tickers(tickers).await?;
        
        ws.run(event_tx).await?;

        Ok(())
    }
}
