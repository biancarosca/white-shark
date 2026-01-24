use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::exchanges::kalshi::{
    KalshiEvent, KalshiMarketStatus, KalshiOrderbook, KalshiOrderbookDelta, KalshiTicker, OrderbookLevel,
};
use crate::exchanges::PriceUpdate;
use crate::state::KalshiState;

pub async fn process_events(
    mut binance_rx: mpsc::Receiver<PriceUpdate>,
    mut kalshi_rx: mpsc::Receiver<KalshiEvent>,
    state: KalshiState,
) {
    info!("Starting event processor...");

    loop {
        tokio::select! {
            Some(event) = kalshi_rx.recv() => {
                handle_kalshi_event(event, &state);
            }
            Some(price) = binance_rx.recv() => {
                handle_binance_price(price);
            }
            else => {
                warn!("All channels closed, stopping event processor");
                break;
            }
        }
    }
}

fn handle_kalshi_event(event: KalshiEvent, state: &KalshiState) {
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
            handle_orderbook_update(ob, state);
        }
        KalshiEvent::OrderbookDelta(delta) => {
            handle_orderbook_delta(delta, state);
        }
        KalshiEvent::Trade(trade) => {
            info!(
                "💰 Kalshi {} trade | yes: {:?}, no: {:?}",
                trade.market_ticker, trade.yes_price, trade.no_price
            );
        }
    }
}

fn handle_orderbook_update(ob: KalshiOrderbook, state: &KalshiState) {
    // Update orderbook state (DashMap handles concurrency automatically)
    let mut existing = state.orderbooks.entry(ob.market_ticker.clone()).or_insert_with(|| {
        KalshiOrderbook {
            market_ticker: ob.market_ticker.clone(),
            yes_bids: Vec::new(),
            yes_asks: Vec::new(),
            no_bids: Vec::new(),
            no_asks: Vec::new(),
        }
    });
    
    // Snapshot/update replaces the stored levels
    existing.yes_bids = ob.yes_bids.clone();
    existing.no_bids = ob.no_bids.clone();
    
    // Sort bids descending
    existing.yes_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
    existing.no_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));

    // Derive top asks from opposing side top bid:
    // - Best YES ask is (1 - best NO bid)
    // - Best NO ask is (1 - best YES bid)
    existing.yes_asks.clear();
    existing.no_asks.clear();
    if let Some(best_no_bid) = existing.no_bids.first().map(|l| l.price) {
        existing.yes_asks.push(OrderbookLevel { price: 1.0 - best_no_bid, quantity: 0 });
    }
    if let Some(best_yes_bid) = existing.yes_bids.first().map(|l| l.price) {
        existing.no_asks.push(OrderbookLevel { price: 1.0 - best_yes_bid, quantity: 0 });
    }
    
    // Print top bid and top ask
    let top_bid = existing.yes_bids.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_ask = existing.yes_asks.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());

    let top_bid_no = existing.no_bids.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    let top_ask_no = existing.no_asks.first().map(|l| format!("${:.4}", l.price)).unwrap_or_else(|| "N/A".to_string());
    
    info!(
        "📚 Kalshi {} | Top bid YES: {} | Top ask YES: {} | Top bid NO: {} | Top ask NO: {}",
        existing.market_ticker, top_bid, top_ask, top_bid_no, top_ask_no
    );
}

fn handle_orderbook_delta(delta: KalshiOrderbookDelta, state: &KalshiState) {
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

