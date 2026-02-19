use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep_until, Instant};
use tracing::{error, info, warn};

use super::api::KalshiApi;
use super::auth::KalshiAuth;
use super::context::ClientContext;
use super::handler::MessageHandler;
use super::market_data::MarketDataWriter;
use super::models::KalshiWsMessage;
use super::subscriptions::SubscriptionManager;
use super::utils::{
    maintenance_sleep_duration, 
    next_15min_interval, 
    next_maintenance_start
};
use super::websocket::KalshiWebSocket;
use crate::config::KalshiConfig;
use crate::constants::KALSHI_WS_URL;
use crate::db::main::Db;
use crate::error::{Error, Result};
use crate::exchanges::kalshi::constants::*;
use crate::state::KalshiState;

pub struct KalshiClient {
    auth: Arc<KalshiAuth>,
    api: KalshiApi,
    ws: Option<Arc<Mutex<KalshiWebSocket>>>,
    ctx: ClientContext,
}

impl KalshiClient {
    pub fn new(config: KalshiConfig, db: Arc<Db>) -> Result<Self> {
        let auth = Arc::new(KalshiAuth::create_auth(&config)?);
        let api = KalshiApi::new(auth.clone());

        if config.tracked_symbols.is_empty() {
            return Err(Error::Config("No tracked symbols configured".into()));
        }

        let market_data_tx = MarketDataWriter::spawn(db.clone());
        let ctx = ClientContext::new(config.tracked_symbols, db, market_data_tx);

        Ok(Self { auth, api, ws: None, ctx })
    }

    pub fn state(&self) -> &KalshiState {
        &self.ctx.state
    }

    pub async fn connect(&mut self) -> Result<()> {
        let mut ws = KalshiWebSocket::new(KALSHI_WS_URL, self.auth.clone());
        ws.connect().await?;
        self.ws = Some(Arc::new(Mutex::new(ws)));
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(ws) = &self.ws {
            ws.lock().await.disconnect().await?;
        }
        self.ws = None;
        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        match &self.ws {
            Some(ws) => ws.lock().await.is_connected(),
            None => false,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        let mut backoff_secs = INITIAL_BACKOFF_SECS;

        loop {
            if let Some(sleep_dur) = maintenance_sleep_duration() {
                info!("🛑 Maintenance window active, sleeping for {}s...", sleep_dur.as_secs());
                let _ = self.disconnect().await;
                tokio::time::sleep(sleep_dur).await;
                info!("✅ Maintenance window ended, resuming...");
                backoff_secs = INITIAL_BACKOFF_SECS;
            }

            let (result, was_stable) = self.run_connection_loop().await;

            match result {
                Ok(_) => {
                    info!("WebSocket loop exited cleanly");
                    break;
                }
                Err(e) => {
                    if was_stable {
                        backoff_secs = INITIAL_BACKOFF_SECS;
                    }
                    error!("🔴 WebSocket error: {}. Reconnecting in {}s...", e, backoff_secs);
                    let _ = self.disconnect().await;
                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                }
            }
        }

        Ok(())
    }

    async fn run_connection_loop(&mut self) -> (Result<()>, bool) {
        if let Err(e) = self.connect().await {
            return (Err(e), false);
        }
        info!("🔗 WebSocket connected");

        let ws = match &self.ws {
            Some(ws) => ws.clone(),
            None => return (Err(Error::WebSocket("Not connected".into())), false),
        };

        if let Err(e) = SubscriptionManager::fetch_and_set_all(&mut self.ctx, &self.api).await {
            return (Err(e), false);
        }
        if let Err(e) = SubscriptionManager::subscribe_all(&mut self.ctx, &ws).await {
            return (Err(e), false);
        }

        let (msg_tx, mut msg_rx) = mpsc::channel::<KalshiWsMessage>(100);

        let ws_reader = ws.clone();
        let ws_handle = tokio::spawn(async move {
            loop {
                let msg_result = {
                    let mut guard = ws_reader.lock().await;
                    guard.recv().await
                };

                match msg_result {
                    Ok(Some(msg)) => {
                        if let Err(e) = msg_tx.send(msg).await {
                            error!("Failed to send message to channel: {}", e);
                            break;
                        }
                    }
                    Ok(None) => {
                        if !ws_reader.lock().await.is_connected() {
                            warn!("WebSocket connection lost");
                            break;
                        }
                    }
                    Err(e) => {
                        error!("WebSocket receive error: {}", e);
                        break;
                    }
                }
            }
        });

        info!("🧠 Starting message processing loop");

        let mut received_messages = false;
        let mut fetch_deadline = next_15min_interval();
        let maintenance_deadline = next_maintenance_start();
        let mut last_message_at = Instant::now();
        let mut idle_deadline = last_message_at + Duration::from_secs(WS_IDLE_RECONNECT_SECS);

        loop {
            tokio::select! {
                maybe_msg = msg_rx.recv() => {
                    match maybe_msg {
                        Some(msg) => {
                            received_messages = true;
                            last_message_at = Instant::now();
                            idle_deadline = last_message_at + Duration::from_secs(WS_IDLE_RECONNECT_SECS);
                            if let Err(e) = MessageHandler::handle(&mut self.ctx, msg).await {
                                error!("Error handling message: {}", e);
                            }
                        }
                        None => {
                            warn!("WebSocket message channel closed");
                            break;
                        }
                    }
                }
                _ = sleep_until(fetch_deadline) => {
                    if let Err(e) = SubscriptionManager::handle_due_markets(&mut self.ctx, &self.api, &ws).await {
                        error!("Error during post-close market fetch: {}", e);
                    }
                    fetch_deadline = next_15min_interval();
                }
                _ = sleep_until(maintenance_deadline) => {
                    info!("🛑 Approaching maintenance window, disconnecting...");
                    break;
                }
                _ = sleep_until(idle_deadline) => {
                    warn!(
                        "No WebSocket message received for {}s, reconnecting (stale connection)",
                        WS_IDLE_RECONNECT_SECS
                    );
                    break;
                }
            }
        }

        let _ = ws_handle.await;
        (Err(Error::WebSocket("Connection lost".into())), received_messages)
    }
}
