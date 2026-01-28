use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{info, warn};
use chrono::{DateTime, Utc};

use crate::exchanges::kalshi::{
    KalshiEvent, KalshiMarketStatus, KalshiOrderbook, KalshiOrderbookDelta, KalshiTicker, OrderbookLevel,
};
use crate::exchanges::PriceUpdate;
use crate::state::KalshiState;

#[derive(Debug, Clone)]
pub struct ImbalanceAlert {
    pub message_received_time: DateTime<Utc>,
    pub imbalance_detected_time: DateTime<Utc>,
    pub symbol: String,
    pub imbalance_top_5: f64,
    pub imbalance_top_10: f64,
    pub imbalance_all: f64,
    pub top_5_bids: f64,
    pub top_5_asks: f64,
    pub top_10_bids: f64,
    pub top_10_asks: f64,
    pub all_bids: f64,
    pub all_asks: f64,
}

pub async fn process_events(
    mut binance_rx: mpsc::Receiver<PriceUpdate>,
    mut kalshi_rx: mpsc::Receiver<KalshiEvent>,
    mut imbalance_rx: mpsc::Receiver<ImbalanceAlert>,
    state: Arc<KalshiState>,
) {
    info!("Starting event processor...");

    // Track active imbalance monitoring sessions
    let active_monitors: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let kalshi_changes: Arc<Mutex<HashMap<String, Vec<(DateTime<Utc>, f64, f64, f64, f64)>>>> = Arc::new(Mutex::new(HashMap::new()));

    loop {
        tokio::select! {
            Some(event) = kalshi_rx.recv() => {
                handle_kalshi_event(event, &state, &active_monitors, &kalshi_changes).await;
            }
            Some(price) = binance_rx.recv() => {
                handle_binance_price(price);
            }
            Some(alert) = imbalance_rx.recv() => {
                handle_imbalance_alert(alert, &state, &active_monitors, &kalshi_changes).await;
            }
            else => {
                warn!("All channels closed, stopping event processor");
                break;
            }
        }
    }
}

async fn handle_imbalance_alert(
    alert: ImbalanceAlert,
    state: &KalshiState,
    active_monitors: &Arc<Mutex<HashMap<String, Instant>>>,
    kalshi_changes: &Arc<Mutex<HashMap<String, Vec<(DateTime<Utc>, f64, f64, f64, f64)>>>>,
) {
    // Get the first tracked Kalshi market - must be tracked to proceed
    let kalshi_ticker = match state.tracked_markets.iter().next() {
        Some(entry) => entry.key().clone(),
        None => {
            warn!("No tracked Kalshi markets available, skipping imbalance alert");
            return;
        }
    };

    // Get current Kalshi prices from orderbook (must exist for tracked markets)
    let orderbook = match state.get_orderbook(&kalshi_ticker) {
        Some(ob) => ob,
        None => {
            warn!("No orderbook available for tracked market {}, skipping imbalance alert", kalshi_ticker);
            return;
        }
    };
    
    let yes_ask = orderbook.yes_asks.first().map(|l| l.price);
    let no_ask = orderbook.no_asks.first().map(|l| l.price);
    let yes_bid = orderbook.yes_bids.first().map(|l| l.price);
    let no_bid = orderbook.no_bids.first().map(|l| l.price);

    // Check if there's already an active monitor - if so, skip this alert
    let now = Instant::now();
    let monitors_guard = active_monitors.lock().await;
    let has_active_monitor = monitors_guard.iter().any(|(_, start_time)| {
        now.duration_since(*start_time) <= Duration::from_secs(15)
    });
    drop(monitors_guard);

    if has_active_monitor {
        info!(
            "⏸️  IMBALANCE ALERT SKIPPED - Active monitor already running\n\
            Message received: {}\n\
            Imbalance detected: {}\n\
            Binance symbol: {}\n\
            Imbalance ratios - Top 5: {:.3}, Top 10: {:.3}, All: {:.3}",
            alert.message_received_time,
            alert.imbalance_detected_time,
            alert.symbol,
            alert.imbalance_top_5,
            alert.imbalance_top_10,
            alert.imbalance_all,
        );
        return;
    }

    // Log the imbalance alert with all information
    info!(
        "🚨 IMBALANCE ALERT 🚨\n\
        Message received: {}\n\
        Imbalance detected: {}\n\
        Binance symbol: {}\n\
        Imbalance ratios - Top 5: {:.3}, Top 10: {:.3}, All: {:.3}\n\
        Quantities - Top 5: bids={:.2}, asks={:.2} | Top 10: bids={:.2}, asks={:.2} | All: bids={:.2}, asks={:.2}\n\
        Kalshi market: {}\n\
        Current YES ask: {}\n\
        Current NO ask: {}\n\
        Current YES bid: {}\n\
        Current NO bid: {}\n\
        --- Starting 15-second Kalshi odds tracking ---",
        alert.message_received_time,
        alert.imbalance_detected_time,
        alert.symbol,
        alert.imbalance_top_5,
        alert.imbalance_top_10,
        alert.imbalance_all,
        alert.top_5_bids,
        alert.top_5_asks,
        alert.top_10_bids,
        alert.top_10_asks,
        alert.all_bids,
        alert.all_asks,
        kalshi_ticker,
        yes_ask.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string()),
        no_ask.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string()),
        yes_bid.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string()),
        no_bid.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string()),
    );

    // Initialize tracking for this alert
    let monitor_key = format!("{}_{}", alert.symbol, alert.imbalance_detected_time.timestamp());
    {
        let mut monitors_guard = active_monitors.lock().await;
        monitors_guard.insert(monitor_key.clone(), Instant::now());
    }
    {
        let mut changes = kalshi_changes.lock().await;
        changes.insert(monitor_key.clone(), Vec::new());

        // Record initial state
        if let (Some(ya), Some(na), Some(yb), Some(nb)) = (yes_ask, no_ask, yes_bid, no_bid) {
            changes.get_mut(&monitor_key).unwrap().push((
                Utc::now(),
                ya,
                na,
                yb,
                nb,
            ));
        }
    }

    // Capture initial prices for file writing
    let initial_yes_ask = yes_ask.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string());
    let initial_yes_bid = yes_bid.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string());
    let initial_no_ask = no_ask.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string());
    let initial_no_bid = no_bid.map(|p| format!("${:.4}", p)).unwrap_or_else(|| "N/A".to_string());

    // Spawn a task to stop monitoring after 15 seconds
    let monitor_key_clone = monitor_key.clone();
    let kalshi_changes_clone = kalshi_changes.clone();
    let active_monitors_clone = active_monitors.clone();
    let alert_clone = alert.clone();
    let kalshi_ticker_clone = kalshi_ticker.clone();
    let initial_yes_ask_clone = initial_yes_ask.clone();
    let initial_yes_bid_clone = initial_yes_bid.clone();
    let initial_no_ask_clone = initial_no_ask.clone();
    let initial_no_bid_clone = initial_no_bid.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(15)).await;
        
        // Remove this monitor from active monitors
        {
            let mut monitors_guard = active_monitors_clone.lock().await;
            monitors_guard.remove(&monitor_key_clone);
        }
        
        let changes_guard = kalshi_changes_clone.lock().await;
        if let Some(changes) = changes_guard.get(&monitor_key_clone) {
            let changes_summary = changes.iter().enumerate().map(|(i, (time, ya, na, yb, nb))| {
                format!(
                    "  [{}] {} - YES: ask={:.4}, bid={:.4} | NO: ask={:.4}, bid={:.4}",
                    i + 1,
                    time.format("%H:%M:%S%.3f"),
                    ya, yb, na, nb
                )
            }).collect::<Vec<_>>().join("\n");

            info!(
                "📊 Kalshi odds tracking completed for imbalance alert ({} changes recorded)\n\
                Changes:\n{}",
                changes.len(),
                changes_summary
            );

            // Write to file
            let filename = format!(
                "imbalance_{}_{}.txt",
                alert_clone.symbol,
                alert_clone.imbalance_detected_time.format("%Y%m%d_%H%M%S")
            );
            
            let file_content = format!(
                "IMBALANCE ALERT REPORT\n\
                =====================\n\n\
                Message received: {}\n\
                Imbalance detected: {}\n\
                Binance symbol: {}\n\
                Kalshi market: {}\n\n\
                IMBALANCE RATIOS:\n\
                - Top 5: {:.3}\n\
                - Top 10: {:.3}\n\
                - All: {:.3}\n\n\
                QUANTITIES:\n\
                - Top 5: bids={:.2}, asks={:.2}\n\
                - Top 10: bids={:.2}, asks={:.2}\n\
                - All: bids={:.2}, asks={:.2}\n\n\
                INITIAL KALSHI PRICES:\n\
                - YES ask: {}\n\
                - YES bid: {}\n\
                - NO ask: {}\n\
                - NO bid: {}\n\n\
                KALSHI ODDS CHANGES ({} total):\n\
                {}\n",
                alert_clone.message_received_time,
                alert_clone.imbalance_detected_time,
                alert_clone.symbol,
                kalshi_ticker_clone,
                alert_clone.imbalance_top_5,
                alert_clone.imbalance_top_10,
                alert_clone.imbalance_all,
                alert_clone.top_5_bids,
                alert_clone.top_5_asks,
                alert_clone.top_10_bids,
                alert_clone.top_10_asks,
                alert_clone.all_bids,
                alert_clone.all_asks,
                initial_yes_ask_clone,
                initial_yes_bid_clone,
                initial_no_ask_clone,
                initial_no_bid_clone,
                changes.len(),
                changes_summary
            );

            match tokio::fs::write(&filename, file_content).await {
                Ok(_) => {
                    info!("✅ Imbalance alert data written to file: {}", filename);
                }
                Err(e) => {
                    warn!("❌ Failed to write imbalance alert to file {}: {}", filename, e);
                }
            }
        }
    });
}

async fn handle_kalshi_event(
    event: KalshiEvent,
    state: &KalshiState,
    active_monitors: &Arc<Mutex<HashMap<String, Instant>>>,
    kalshi_changes: &Arc<Mutex<HashMap<String, Vec<(DateTime<Utc>, f64, f64, f64, f64)>>>>,
) {
    match event {
        KalshiEvent::MarketStatusChanged {
            ticker,
            old_status,
            new_status,
        } => {
            info!(
                "📊 Kalshi {} status: {:?} -> {}",
                ticker, old_status, new_status
            );

            if new_status == KalshiMarketStatus::Open.as_str() {
                info!("🟢 Market {} opened!", ticker);
            } else if new_status == KalshiMarketStatus::Closed.as_str() {
                info!("🔴 Market {} closed, find next one...", ticker);
            }
        }
        KalshiEvent::TickerUpdate(ticker) => {
            handle_ticker_update(&ticker);
        }
        KalshiEvent::OrderbookUpdate(ob) => {
            handle_orderbook_update(ob, state, active_monitors, kalshi_changes).await;
        }
        KalshiEvent::OrderbookDelta(delta) => {
            handle_orderbook_delta(delta, state, active_monitors, kalshi_changes).await;
        }
        KalshiEvent::Trade(trade) => {
            info!(
                "💰 Kalshi {} trade | yes: {:?}, no: {:?}",
                trade.market_ticker, trade.yes_price, trade.no_price
            );
        }
    }
}

async fn handle_orderbook_update(
    ob: KalshiOrderbook,
    state: &KalshiState,
    active_monitors: &Arc<Mutex<HashMap<String, Instant>>>,
    kalshi_changes: &Arc<Mutex<HashMap<String, Vec<(DateTime<Utc>, f64, f64, f64, f64)>>>>,
) {
    if !state.tracked_markets.contains_key(&ob.market_ticker) {
       info!("Market {} not tracked, skipping orderbook update", ob.market_ticker);
       return;
    }
    
    let mut existing = state.orderbooks.entry(ob.market_ticker.clone()).or_insert_with(|| {
        KalshiOrderbook {
            market_ticker: ob.market_ticker.clone(),
            yes_bids: Vec::new(),
            yes_asks: Vec::new(),
            no_bids: Vec::new(),
            no_asks: Vec::new(),
        }
    });
    
    existing.yes_bids = ob.yes_bids.clone();
    existing.no_bids = ob.no_bids.clone();
    
    existing.yes_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
    existing.no_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));

    existing.yes_asks.clear();
    existing.no_asks.clear();
    if let Some(best_no_bid) = existing.no_bids.first().map(|l| l.price) {
        existing.yes_asks.push(OrderbookLevel { price: 1.0 - best_no_bid, quantity: 0 });
    }
    if let Some(best_yes_bid) = existing.yes_bids.first().map(|l| l.price) {
        existing.no_asks.push(OrderbookLevel { price: 1.0 - best_yes_bid, quantity: 0 });
    }
    
    let top_bid = existing.yes_bids.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_ask = existing.yes_asks.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());

    let top_bid_no = existing.no_bids.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_ask_no = existing.no_asks.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    
    info!(
        "📚 Kalshi {} | Top bid YES: {} | Top ask YES: {} | Top bid NO: {} | Top ask NO: {}",
        existing.market_ticker, top_bid, top_ask, top_bid_no, top_ask_no
    );

    // Record change if we're monitoring - need to clone the orderbook since we can't pass RefMut
    let orderbook_clone = existing.clone();
    let ticker_clone = existing.market_ticker.clone();
    record_kalshi_change(&ticker_clone, &orderbook_clone, active_monitors, kalshi_changes).await;
}

async fn handle_orderbook_delta(
    delta: KalshiOrderbookDelta,
    state: &KalshiState,
    active_monitors: &Arc<Mutex<HashMap<String, Instant>>>,
    kalshi_changes: &Arc<Mutex<HashMap<String, Vec<(DateTime<Utc>, f64, f64, f64, f64)>>>>,
) {
    let price = match delta.price_dollars.parse::<f64>() {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to parse delta price '{}': {}", delta.price_dollars, e);
            return;
        }
    };

    let mut existing = state
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

    // Keep bids sorted (desc)
    existing
        .yes_bids
        .sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
    existing
        .no_bids
        .sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));

    // Refresh derived asks
    existing.yes_asks.clear();
    existing.no_asks.clear();
    if let Some(best_no_bid) = existing.no_bids.first().map(|l| l.price) {
        existing.yes_asks.push(OrderbookLevel {
            price: 1.0 - best_no_bid,
            quantity: 0,
        });
    }
    if let Some(best_yes_bid) = existing.yes_bids.first().map(|l| l.price) {
        existing.no_asks.push(OrderbookLevel {
            price: 1.0 - best_yes_bid,
            quantity: 0,
        });
    }

    // Print top bid and top ask after delta update
    let top_bid = existing.yes_bids.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_ask = existing.yes_asks.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_bid_no = existing.no_bids.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_ask_no = existing.no_asks.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    
    info!(
        "📚 Kalshi {} | Top bid YES: {} | Top ask YES: {} | Top bid NO: {} | Top ask NO: {}",
        existing.market_ticker, top_bid, top_ask, top_bid_no, top_ask_no
    );

    // Record change if we're monitoring
    record_kalshi_change(&existing.market_ticker, &*existing, active_monitors, kalshi_changes).await;
}

async fn record_kalshi_change(
    market_ticker: &str,
    orderbook: &KalshiOrderbook,
    active_monitors: &Arc<Mutex<HashMap<String, Instant>>>,
    kalshi_changes: &Arc<Mutex<HashMap<String, Vec<(DateTime<Utc>, f64, f64, f64, f64)>>>>,
) {
    // Get current prices
    let yes_ask = orderbook.yes_asks.first().map(|l| l.price);
    let no_ask = orderbook.no_asks.first().map(|l| l.price);
    let yes_bid = orderbook.yes_bids.first().map(|l| l.price);
    let no_bid = orderbook.no_bids.first().map(|l| l.price);

    if let (Some(ya), Some(na), Some(yb), Some(nb)) = (yes_ask, no_ask, yes_bid, no_bid) {
        // Record for all active monitors that are still within 15 seconds
        let now = Instant::now();
        let monitors_guard = active_monitors.lock().await;
        
        // Check if we have any active monitors
        if monitors_guard.is_empty() {
            return;
        }
        
        let active_keys: Vec<String> = monitors_guard.iter()
            .filter(|(_, start_time)| now.duration_since(**start_time) <= Duration::from_secs(15))
            .map(|(key, _)| key.clone())
            .collect();
        drop(monitors_guard);
        
        let mut changes_guard = kalshi_changes.lock().await;
        for key in active_keys {
            if let Some(changes) = changes_guard.get_mut(&key) {
                // Only record if prices actually changed
                if changes.is_empty() || {
                    let last = changes.last().unwrap();
                    (last.1 - ya).abs() > 1e-6 || (last.2 - na).abs() > 1e-6 ||
                    (last.3 - yb).abs() > 1e-6 || (last.4 - nb).abs() > 1e-6
                } {
                    changes.push((Utc::now(), ya, na, yb, nb));
                    info!(
                        "📈 Kalshi odds change recorded for active imbalance monitor | {} | YES: ask={:.4}, bid={:.4} | NO: ask={:.4}, bid={:.4}",
                        market_ticker, ya, yb, na, nb
                    );
                }
            }
        }
    }
}

fn handle_ticker_update(ticker: &KalshiTicker) {
    let yes_bid = ticker.yes_bid_f64().map(|v| format!("${:.4}", v)).unwrap_or_default();
    let yes_ask = ticker.yes_ask_f64().map(|v| format!("${:.4}", v)).unwrap_or_default();

    info!(
        "📈 Kalshi {} | YES bid: {} | YES ask: {}",
        ticker.market_ticker, yes_bid, yes_ask
    );
}

fn handle_binance_price(update: PriceUpdate) {
    let price_str = if let Some(p) = update.last_price {
        format!("${:.2}", p)
    } else if let (Some(bid), Some(ask)) = (update.bid, update.ask) {
        format!("bid: ${:.2} / ask: ${:.2}", bid, ask)
    } else {
        "N/A".to_string()
    };

    info!("💹 Binance {} | {}", update.symbol, price_str);
}

