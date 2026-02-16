use std::sync::Arc;
use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, sleep_until, Instant as TokioInstant};
use tracing::{info, warn, error};

use super::api::KalshiApi;
use super::auth::KalshiAuth;
use super::models::*;
use super::websocket::KalshiWebSocket;
use crate::config::KalshiConfig;
use crate::db::main::Db;
use crate::error::{Error, Result};
use crate::constants::KALSHI_WS_URL;
use crate::state::KalshiState;
use crate::exchanges::kalshi::{KalshiOrderbook, OrderbookLevel};

#[derive(Clone)]
struct MarketDataUpdate {
    ticker: String,
    asset: String,
    timestamp: DateTime<Utc>,
    yes_ask: f64,
    yes_bid: f64,
    no_ask: f64,
    no_bid: f64,
}

pub struct KalshiClient {
    config: KalshiConfig,
    api: KalshiApi,
    ws: Option<Arc<Mutex<KalshiWebSocket>>>,
    pub state: KalshiState,
    /// Maps series_ticker -> current active market for that series
    current_markets: HashMap<String, KalshiMarket>,
    /// Reverse lookup: market_ticker -> series_ticker (survives market rotation)
    market_to_series: HashMap<String, String>,
    /// All series tickers we're tracking (e.g., ["BTC15M", "ETH15M"])
    series_tickers: Vec<String>,
    subscription_ids: HashMap<String, u64>,
    db: Arc<Db>,
    market_data_tx: mpsc::Sender<MarketDataUpdate>,
}

const BATCH_SIZE: usize = 1000;
const FLUSH_INTERVAL_MS: u64 = 5000;  // Flush every 5 seconds
const CHANNEL_BUFFER_SIZE: usize = 50000;
const FETCH_AFTER_CLOSE_SECS: i64 = 10;

const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64 = 60;

impl KalshiClient {
    fn create_auth(config: &KalshiConfig) -> Result<KalshiAuth> {
        if let Some(ref pem_content) = config.private_key {
            KalshiAuth::from_pem_content(&config.api_key_id, pem_content)
        } else if let Some(ref path) = config.private_key_path {
            KalshiAuth::from_file(&config.api_key_id, path)
        } else {
            Err(Error::Config("No private key configured".into()))
        }
    }

    pub fn new(config: KalshiConfig, db: Arc<Db>) -> Result<Self> {
        let auth = Self::create_auth(&config)?;
        let auth_arc = Arc::new(auth);
        let api = KalshiApi::new(auth_arc.clone());

        if config.tracked_symbols.is_empty() {
            return Err(Error::Config("No tracked symbols configured".into()));
        }
        let series_tickers = config.tracked_symbols.clone();

        let (market_data_tx, market_data_rx) = mpsc::channel::<MarketDataUpdate>(CHANNEL_BUFFER_SIZE);
        
        Self::spawn_market_data_writer(db.clone(), market_data_rx);

        Ok(Self {
            config,
            api,
            ws: None,
            state: KalshiState::new(),
            current_markets: HashMap::new(),
            market_to_series: HashMap::new(),
            series_tickers,
            subscription_ids: HashMap::new(),
            db,
            market_data_tx,
        })
    }

    fn spawn_market_data_writer(db: Arc<Db>, mut rx: mpsc::Receiver<MarketDataUpdate>) {
        tokio::spawn(async move {
            let mut batch: Vec<MarketDataUpdate> = Vec::with_capacity(BATCH_SIZE);
            let mut flush_interval = interval(Duration::from_millis(FLUSH_INTERVAL_MS));

            loop {
                tokio::select! {
                    maybe_update = rx.recv() => {
                        match maybe_update {
                            Some(update) => {
                                batch.push(update);
                                if batch.len() >= BATCH_SIZE {
                                    Self::flush_batch(&db, &mut batch).await;
                                }
                            }
                            None => {
                                if !batch.is_empty() {
                                    Self::flush_batch(&db, &mut batch).await;
                                }
                                info!("Market data writer shutting down");
                                break;
                            }
                        }
                    }
                    _ = flush_interval.tick() => {
                        if !batch.is_empty() {
                            Self::flush_batch(&db, &mut batch).await;
                        }
                    }
                }
            }
        });
    }

    async fn flush_batch(db: &Db, batch: &mut Vec<MarketDataUpdate>) {
        if batch.is_empty() {
            return;
        }

        let count = batch.len();
        let records: Vec<_> = batch
            .drain(..)
            .map(|u| (u.ticker, u.asset, u.timestamp, u.yes_ask, u.yes_bid, u.no_ask, u.no_bid))
            .collect();

        // if let Err(e) = db.insert_market_data_batch(records).await {
        //     error!("Failed to batch insert market data: {}", e);
        // } else {
        //     info!("📝 Flushed {} market data records to DB", count);
        // }
    }

    pub async fn connect(&mut self) -> Result<()> {
        let auth = Self::create_auth(&self.config)?;
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
        let mut backoff_secs = INITIAL_BACKOFF_SECS;

        loop {
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

        if let Err(e) = self.fetch_and_set_all_markets().await {
            return (Err(e), false);
        }
        if let Err(e) = self.subscribe_to_all_markets().await {
            return (Err(e), false);
        }

        let (msg_tx, mut msg_rx) = mpsc::channel::<KalshiWsMessage>(100);

        let ws = match self.ws.as_ref() {
            Some(ws) => ws.clone(),
            None => return (Err(Error::WebSocket("Not connected".into())), false),
        };

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
        
        let mut received_messages = false;
        
        let mut fetch_deadline = self.next_15min_interval();

        loop {
            tokio::select! {
                maybe_msg = msg_rx.recv() => {
                    match maybe_msg {
                        Some(msg) => {
                            received_messages = true;
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
                _ = sleep_until(fetch_deadline) => {
                    if let Err(e) = self.handle_due_markets().await {
                        error!("Error during post-close market fetch: {}", e);
                    }

                    fetch_deadline = self.next_15min_interval();
                }
            }
        }

        let _ = ws_handle.await;
        
        (Err(Error::WebSocket("Connection lost".into())), received_messages)
    }

    async fn handle_message(&mut self, msg: KalshiWsMessage) -> Result<()> {
        if msg.is_subscribed() {
            if let Some(sid) = msg.payload().and_then(|p| p.get("sid")).and_then(|s| s.as_u64()) {
                let channel = msg.payload().and_then(|p| p.get("channel")).and_then(|c| c.as_str());
                match channel {
                    Some(c) => {
                        info!("✅ Subscription confirmed: sid={}, channel={}", sid, c);
                        self.subscription_ids.insert(c.to_string(), sid);
                    }
                    None => {
                        error!("No channel found in subscription message");
                        return Ok(());
                    }
                }
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

        // Look up series from current_markets first, fall back to market_to_series
        // (the timer may have already rotated the market out of current_markets)
        let series_ticker = self.current_markets.iter()
            .find(|(_, market)| market.ticker == lifecycle_msg.market_ticker)
            .map(|(series, _)| series.clone())
            .or_else(|| self.market_to_series.get(&lifecycle_msg.market_ticker).cloned());

        let series_ticker = match series_ticker {
            Some(s) => s,
            None => return Ok(()),
        };

        info!("📊 Received market lifecycle event for {} (series {}): {:?}", 
              lifecycle_msg.market_ticker, series_ticker, lifecycle_msg);

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
            if let Some(result) = &lifecycle_msg.result {
                let strike_price = self.current_markets.get(&series_ticker)
                    .and_then(|m| m.extra.get("floor_strike"))
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        let tracked = self.state.tracked_markets.get(&lifecycle_msg.market_ticker)?;
                        tracked.extra.get("floor_strike")?.as_f64()
                    });

                // if let Err(e) = self.db.insert_market_info(
                //     &lifecycle_msg.market_ticker,
                //     Utc::now(),
                //     strike_price,
                //     result,
                // ).await {
                //     error!("Failed to insert market info: {}", e);
                // }
            }

            info!("🔴 Market {} closed (lifecycle), unsubscribing for series {}...", 
                  lifecycle_msg.market_ticker, series_ticker);

            self.market_to_series.remove(&lifecycle_msg.market_ticker);

            let is_still_current = self.current_markets.get(&series_ticker)
                .map(|m| m.ticker == lifecycle_msg.market_ticker)
                .unwrap_or(false);
            if is_still_current {
                self.current_markets.remove(&series_ticker);
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
            "📚 Kalshi {} | Top bid YES: {} | Top ask YES: {} | Top bid NO: {} | Top ask NO: {} at time: {}",
            ob.market_ticker, top_bid, top_ask, top_bid_no, top_ask_no, Utc::now()
        );
    }

    fn queue_market_data_update(&self, ob: &KalshiOrderbook) {
        let asset = self.current_markets.iter()
            .find(|(_, market)| market.ticker == ob.market_ticker)
            .map(|(series, _)| series.clone())
            .or_else(|| self.market_to_series.get(&ob.market_ticker).cloned());

        let asset = match asset {
            Some(s) => s,
            None => {
                error!("No series ticker found for market: {}", ob.market_ticker);
                return;
            }
        };

        let update = MarketDataUpdate {
            ticker: ob.market_ticker.clone(),
            asset,
            timestamp: Utc::now(),
            yes_ask: ob.yes_asks.first().map(|l| l.price).unwrap_or(0.0),
            yes_bid: ob.yes_bids.first().map(|l| l.price).unwrap_or(0.0),
            no_ask: ob.no_asks.first().map(|l| l.price).unwrap_or(0.0),
            no_bid: ob.no_bids.first().map(|l| l.price).unwrap_or(0.0),
        };

        if let Err(e) = self.market_data_tx.try_send(update) {
            error!("Failed to queue market data update: {}", e);
        }
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

        // Queue for batched database insert
        self.queue_market_data_update(&existing);
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

        // Queue for batched database insert
        self.queue_market_data_update(&existing);
    }

    async fn subscribe_to_all_markets(&mut self) -> Result<()> {
        if self.current_markets.is_empty() {
            return Err(Error::Other("No current markets set".into()));
        }

        let tickers: Vec<String> = self.current_markets.iter()
            .map(|(_, m)| m.ticker.clone())
            .collect();

        let ws = self.ws.as_ref()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))?;
        let mut ws_guard = ws.lock().await;

        if !self.subscription_ids.is_empty() {
            let orderbook_sid = self.subscription_ids.get("orderbook_delta").cloned();
            if let Some(sid) = orderbook_sid {
                info!("⛓️‍💥 Unsubscribing from orderbook with sid: {:?}", sid);
                ws_guard.unsubscribe(vec![sid]).await?;
            }
            
            self.subscription_ids.remove("orderbook_delta");
        }

        info!("📡 Subscribing to {} markets: {:?}", tickers.len(), tickers);

        ws_guard.subscribe_orderbook(tickers.clone()).await?;

        if self.subscription_ids.get("market_lifecycle_v2").is_none() {
            info!("🤝 Subscribing to market lifecycle");
            ws_guard.subscribe_market_lifecycle().await?;
        }

        Ok(())
    }

    async fn fetch_and_set_all_markets(&mut self) -> Result<()> {
        for series_ticker in &self.series_tickers.clone() {
            let markets = self.api.fetch_market_by_ticker(series_ticker, Some("open")).await?;
            
            if markets.is_empty() {
                warn!("No open markets found for series: {}", series_ticker);
                continue;
            }

            let next_market = markets[0].clone();
            
            if let Some(old_market) = self.current_markets.get(series_ticker) {
                info!("🔄 Replacing market {} with {} for series {}", old_market.ticker, next_market.ticker, series_ticker);
            } else {
                info!("📡 Setting initial market for {}: {}", series_ticker, next_market.ticker);
            }
            
            self.current_markets.insert(series_ticker.clone(), next_market.clone());
            self.market_to_series.insert(next_market.ticker.clone(), series_ticker.clone());
            self.track_market(&next_market);

            if let Some(floor_strike) = next_market.extra.get("floor_strike") {
                info!("💰 Floor strike for {}: {}", next_market.ticker, floor_strike);
            }
        }

        if self.current_markets.is_empty() {
            return Err(Error::Other("No open markets found for any tracked series".into()));
        }

        Ok(())
    }

    fn next_15min_interval(&self) -> TokioInstant {
        let now = Utc::now();
        let seconds_since_hour = now.timestamp() % 3600;
        let seconds_into_15min_block = seconds_since_hour % 900;
        
        let seconds_until_next_15min = if seconds_into_15min_block == 0 {
            FETCH_AFTER_CLOSE_SECS as u64
        } else {
            (900 - seconds_into_15min_block) as u64 + FETCH_AFTER_CLOSE_SECS as u64
        };

        TokioInstant::now() + Duration::from_secs(seconds_until_next_15min)
    }

    async fn handle_due_markets(&mut self) -> Result<()> {
        info!("⏰ 15-minute interval reached, rotating all markets...");
        
        if let Err(e) = self.fetch_and_set_all_markets().await {
            error!("Failed to fetch all markets: {}", e);
            return Err(e);
        }
        
        self.subscribe_to_all_markets().await?;
            
        Ok(())
    }
}

