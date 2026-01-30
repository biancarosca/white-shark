use std::sync::Arc;
use std::collections::HashMap;

use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn, error};

use super::api::KalshiApi;
use super::auth::KalshiAuth;
use super::models::*;
use super::websocket::KalshiWebSocket;
use crate::config::KalshiConfig;
use crate::error::{Error, Result};
use crate::constants::KALSHI_WS_URL;
use crate::state::KalshiState;
use crate::exchanges::kalshi::{KalshiOrderbook, OrderbookLevel};

pub struct KalshiClient {
    config: KalshiConfig,
    api: KalshiApi,
    ws: Option<Arc<Mutex<KalshiWebSocket>>>,
    pub state: KalshiState,
    current_market: Option<KalshiMarket>,
    series_ticker: String,
    subscription_ids: HashMap<String, Vec<u64>>, // Track SIDs by ticker/channel
}

impl KalshiClient {
    pub fn new(config: KalshiConfig) -> Result<Self> {
        let auth = KalshiAuth::from_file(&config.api_key_id, &config.private_key_path)?;
        let auth_arc = Arc::new(auth);
        let api = KalshiApi::new(auth_arc.clone());

        let series_ticker = config.tracked_symbols.first()
            .ok_or_else(|| Error::Config("No tracked symbols configured".into()))?
            .clone();

        Ok(Self {
            config,
            api,
            ws: None,
            state: KalshiState::new(),
            current_market: None,
            series_ticker,
            subscription_ids: HashMap::new(),
        })
    }

    pub async fn connect(&mut self) -> Result<()> {
        let auth = KalshiAuth::from_file(&self.config.api_key_id, &self.config.private_key_path)?;
        let mut ws = KalshiWebSocket::new(KALSHI_WS_URL, auth);
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
        if let Some(ws) = &self.ws {
            ws.lock().await.is_connected()
        } else {
            false
        }
    }

    fn track_market(&self, market: &KalshiMarket) {
        info!("🪄 Tracking market: {} ({:?})", market.ticker, market.status);
        self.state.tracked_markets.insert(market.ticker.clone(), market.clone());
    }

    pub async fn start(&mut self) -> Result<()> {
        if !self.is_connected().await {
            self.connect().await?;
        }
        
        self.fetch_and_set_next_market().await?;
        self.subscribe_to_current_market().await?;

        let (msg_tx, mut msg_rx) = mpsc::channel::<KalshiWsMessage>(100);

        let ws = self.ws.as_ref()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))?
            .clone();

        let ws_handle = tokio::spawn(async move {
            loop {
                let msg_result = {
                    let mut ws_guard = ws.lock().await;
                    ws_guard.recv().await
                };

                match msg_result {
                    Ok(Some(msg)) => {
                        if let Err(e) = msg_tx.send(msg).await {
                            error!("Failed to send message to state manager: {}", e);
                            break;
                        }
                    }
                    Ok(None) => {
                        let is_connected = ws.lock().await.is_connected();
                        if !is_connected {
                            warn!("WebSocket connection lost");
                            break;
                        }
                        continue;
                    }
                    Err(e) => {
                        error!("WebSocket receive error: {}", e);
                        break;
                    }
                }
            }
        });

        info!("🧠 Starting state manager message processing");
        loop {
            match msg_rx.recv().await {
                Some(msg) => {
                    if let Err(e) = self.handle_message(msg).await {
                        error!("Error handling message: {}", e);
                    }
                }
                None => {
                    warn!("WebSocket message channel closed");
                    break;
                }
            }
        }

        let _ = ws_handle.await;
        Ok(())
    }

    async fn handle_message(&mut self, msg: KalshiWsMessage) -> Result<()> {
        if msg.is_subscribed() {
            let sid = msg.payload()
                .and_then(|p| p.get("sid"))
                .and_then(|s| s.as_u64());
            
            if let Some(sid) = sid {
                self.subscription_ids.entry(self.series_ticker.clone()).or_insert_with(Vec::new).push(sid);
                info!("✅ Subscription confirmed: sid={}", sid);
            }
            return Ok(());
        }

        if let Some(error) = &msg.error {
            error!("WebSocket error: {}", error);
            return Ok(());
        }

        if msg.msg_type.as_deref() == Some("error") {
            if let Some(payload) = msg.payload() {
                error!("WebSocket error message: {:?}", payload);
            }
            return Ok(());
        }

        let payload = match msg.payload() {
            Some(p) => p,
            None => return Ok(()),
        };

        match msg.msg_type.as_deref() {
            Some("orderbook_snapshot") => {
                self.handle_orderbook_snapshot(payload.clone()).await?;
            }
            Some("orderbook_delta") => {
                self.handle_orderbook_delta(payload.clone()).await?;
            }
            Some("market_lifecycle_v2") => {
                self.handle_market_lifecycle(payload.clone()).await?;
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_orderbook_snapshot(&self, payload: serde_json::Value) -> Result<()> {
        match serde_json::from_value::<KalshiOrderbookSnapshot>(payload.clone()) {
            Ok(snapshot) => {
                let mut yes_levels = Vec::with_capacity(snapshot.yes_dollars.len());
                for (p, q) in snapshot.yes_dollars {
                    if let Ok(price) = p.parse::<f64>() {
                        yes_levels.push(OrderbookLevel { price, quantity: q });
                    }
                }
                let mut no_levels = Vec::with_capacity(snapshot.no_dollars.len());
                for (p, q) in snapshot.no_dollars {
                    if let Ok(price) = p.parse::<f64>() {
                        no_levels.push(OrderbookLevel { price, quantity: q });
                    }
                }

                let ob = KalshiOrderbook {
                    market_ticker: snapshot.market_ticker.clone(),
                    yes_bids: yes_levels,
                    yes_asks: Vec::new(),
                    no_bids: no_levels,
                    no_asks: Vec::new(),
                };

                info!("📸 Received orderbook snapshot for {} ({} YES levels, {} NO levels)", 
                      snapshot.market_ticker, ob.yes_bids.len(), ob.no_bids.len());
                
                self.process_orderbook_update(ob);
                Ok(())
            }
            Err(e) => {
                warn!("Failed to parse orderbook snapshot: {}, payload: {:?}", e, payload);
                Ok(())
            }
        }
    }

    async fn handle_orderbook_delta(&self, payload: serde_json::Value) -> Result<()> {
        match serde_json::from_value::<KalshiOrderbookDelta>(payload.clone()) {
            Ok(delta) => {
                self.process_orderbook_delta(delta);
                Ok(())
            }
            Err(e) => {
                warn!("Failed to parse orderbook delta: {}, payload: {:?}", e, payload);
                Ok(())
            }
        }
    }

    async fn handle_market_lifecycle(&mut self, payload: serde_json::Value) -> Result<()> {
        let lifecycle_msg: KalshiMarketLifecycleMsg = match serde_json::from_value(payload.clone()) {
            Ok(msg) => msg,
            Err(e) => {
                warn!("Failed to parse market lifecycle: {}, payload: {:?}", e, payload);
                return Ok(());
            }
        };

        let current_market_ticker = self.current_market.as_ref().map(|m| m.ticker.clone());
        if current_market_ticker != Some(lifecycle_msg.market_ticker.clone()) {
            return Ok(());
        }

        info!("📊 Received market lifecycle event: {:?}", lifecycle_msg);

        let new_status = match lifecycle_msg.event_type.to_status(lifecycle_msg.is_deactivated) {
            Some(status) => status,
            None => {
                warn!("Failed to map event_type to status: {:?}", lifecycle_msg.event_type);
                return Ok(());
            }
        };

        info!("🔄 Market lifecycle: {} -> {:?} (event: {:?})", 
              lifecycle_msg.market_ticker, new_status, lifecycle_msg.event_type);

        if new_status == KalshiMarketStatus::Closed || new_status == KalshiMarketStatus::Settled {
            if let Some(current) = &self.current_market {
                if current.ticker == lifecycle_msg.market_ticker {
                    info!("🔴 Current market {} closed, switching to next...", current.ticker);
                    self.switch_to_next_market().await?;
                }
            }
        }

        Ok(())
    }

    /// Derives asks from opposite side bids in a binary market
    /// YES Asks = 1.0 - NO Bids, NO Asks = 1.0 - YES Bids
    fn derive_asks_from_bids(ob: &mut KalshiOrderbook) {
        // Collect bids first to avoid borrow checker issues
        let no_bids_copy: Vec<_> = ob.no_bids.clone();
        let yes_bids_copy: Vec<_> = ob.yes_bids.clone();
        
        // YES Asks = derived from NO Bids (1.0 - NO_bid_price)
        ob.yes_asks.clear();
        for no_bid in &no_bids_copy {
            ob.yes_asks.push(OrderbookLevel {
                price: 1.0 - no_bid.price,
                quantity: no_bid.quantity,
            });
        }
        
        // NO Asks = derived from YES Bids (1.0 - YES_bid_price)
        ob.no_asks.clear();
        for yes_bid in &yes_bids_copy {
            ob.no_asks.push(OrderbookLevel {
                price: 1.0 - yes_bid.price,
                quantity: yes_bid.quantity,
            });
        }
    }

    /// Sorts bids and asks appropriately
    fn sort_orderbook(ob: &mut KalshiOrderbook) {
        // Sort bids descending (best bid first)
        ob.yes_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        ob.no_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        
        // Sort asks ascending (best ask first)
        ob.yes_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
        ob.no_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Logs the top bid/ask for both YES and NO sides
    fn log_orderbook_summary(ob: &KalshiOrderbook) {
        let top_bid = ob.yes_bids.first().map(|l| format!("${:.4} @ {}", l.price, l.quantity)).unwrap_or_else(|| "N/A".to_string());
        let top_ask = ob.yes_asks.first().map(|l| format!("${:.4} @ {}", l.price, l.quantity)).unwrap_or_else(|| "N/A".to_string());
        let top_bid_no = ob.no_bids.first().map(|l| format!("${:.4} @ {}", l.price, l.quantity)).unwrap_or_else(|| "N/A".to_string());
        let top_ask_no = ob.no_asks.first().map(|l| format!("${:.4} @ {}", l.price, l.quantity)).unwrap_or_else(|| "N/A".to_string());
        
        info!(
            "📚 Kalshi {} | Top bid YES: {} | Top ask YES: {} | Top bid NO: {} | Top ask NO: {}",
            ob.market_ticker, top_bid, top_ask, top_bid_no, top_ask_no
        );
    }

    fn process_orderbook_update(&self, ob: KalshiOrderbook) {
        // Update orderbook state (DashMap handles concurrency automatically)
        let mut existing = self.state.orderbooks.entry(ob.market_ticker.clone()).or_insert_with(|| {
            KalshiOrderbook {
                market_ticker: ob.market_ticker.clone(),
                yes_bids: Vec::new(),
                yes_asks: Vec::new(),
                no_bids: Vec::new(),
                no_asks: Vec::new(),
            }
        });
        
        // In binary markets, the orderbook snapshot contains:
        // - yes_dollars: All YES bids (people wanting to buy YES)
        // - no_dollars: All NO bids (people wanting to buy NO)
        //
        // We derive asks from the opposite side:
        // - YES Asks = derived from NO Bids (if someone bids 46¢ for NO, they're asking 54¢ for YES)
        // - NO Asks = derived from YES Bids (if someone bids 51¢ for YES, they're asking 49¢ for NO)
        
        // YES Bids and NO Bids are direct from the snapshot
        existing.yes_bids = ob.yes_bids.clone();
        existing.no_bids = ob.no_bids.clone();
        
        // Derive asks from opposite side bids
        Self::derive_asks_from_bids(&mut existing);
        
        // Sort and log
        Self::sort_orderbook(&mut existing);
        Self::log_orderbook_summary(&existing);
    }

    fn process_orderbook_delta(&self, delta: KalshiOrderbookDelta) {
        let price = match delta.price_dollars.parse::<f64>() {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to parse delta price '{}': {}", delta.price_dollars, e);
                return;
            }
        };

        let mut existing = self.state
            .orderbooks
            .entry(delta.market_ticker.clone())
            .or_insert_with(|| KalshiOrderbook {
                market_ticker: delta.market_ticker.clone(),
                yes_bids: Vec::new(),
                yes_asks: Vec::new(),
                no_bids: Vec::new(),
                no_asks: Vec::new(),
            });

        let side = delta.side.to_lowercase();
        let levels = if side == "yes" { &mut existing.yes_bids } else { &mut existing.no_bids };

        // Update quantity at price level (delta can be negative)
        if let Some(idx) = levels.iter().position(|l| (l.price - price).abs() < 1e-12) {
            let new_qty = levels[idx].quantity.saturating_add(delta.delta);
            if new_qty <= 0 {
                levels.remove(idx);
            } else {
                levels[idx].quantity = new_qty;
            }
        } else if delta.delta > 0 {
            levels.push(OrderbookLevel {
                price,
                quantity: delta.delta,
            });
        }

        // Sort bids, then recalculate asks from opposite side
        Self::sort_orderbook(&mut existing);
        Self::derive_asks_from_bids(&mut existing);
        Self::sort_orderbook(&mut existing);
        Self::log_orderbook_summary(&existing);
    }

    async fn subscribe_to_current_market(&mut self) -> Result<()> {
        let current_market = self.current_market.as_ref()
            .ok_or_else(|| Error::Other("No current market set".into()))?;

        let ticker = current_market.ticker.clone();
        info!("📡 Subscribing to market: {}", ticker);

        let ws = self.ws.as_ref()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))?;
        let mut ws_guard = ws.lock().await;
        
        ws_guard.subscribe_orderbook(vec![ticker.clone()]).await?;
        ws_guard.subscribe_market_lifecycle().await?;

        Ok(())
    }

    async fn fetch_and_set_next_market(&mut self) -> Result<()> {
        let markets = self.api.fetch_market_by_ticker(&self.series_ticker, Some("open")).await?;
        
        if markets.is_empty() {
            return Err(Error::Other(format!("No open markets found for series: {}", self.series_ticker)));
        }

        let next_market = markets[0].clone();
        
        if let Some(old_market) = &self.current_market {
            info!("🔄 Replacing market {} with {}", old_market.ticker, next_market.ticker);
        } else {
            info!("📡 Setting initial market: {}", next_market.ticker);
        }
        
        self.current_market = Some(next_market.clone());
        self.track_market(&next_market);

        if let Some(floor_strike) = next_market.extra.get("floor_strike") {
            info!("💰 Floor strike for {}: {}", next_market.ticker, floor_strike);
        }

        Ok(())
    }

    async fn switch_to_next_market(&mut self) -> Result<()> {
        {
            let ws = self.ws.as_ref()
                .ok_or_else(|| Error::WebSocket("Not connected".into()))?;
            let mut ws_guard = ws.lock().await;
            
            let mut sids_to_unsubscribe = Vec::new();
            
            for sids in self.subscription_ids.values() {
                sids_to_unsubscribe.extend_from_slice(sids);
            }
            
            if !sids_to_unsubscribe.is_empty() {
                info!("Unsubscribing from {} subscription(s) using SIDs", sids_to_unsubscribe.len());
                ws_guard.unsubscribe(sids_to_unsubscribe).await?;
                self.subscription_ids.clear();
            } else {
                warn!("No SIDs found for unsubscribe, skipping unsubscribe step");
            }
        }
        
        self.fetch_and_set_next_market().await?;
        self.subscribe_to_current_market().await?;

        Ok(())
    }
}

