use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::exchanges::kalshi::{KalshiEvent, KalshiMarketStatus, KalshiTicker};
use crate::exchanges::PriceUpdate;

pub async fn process_events(
    mut kalshi_rx: mpsc::Receiver<KalshiEvent>,
    // mut binance_rx: mpsc::Receiver<PriceUpdate>,
) {
    info!("Starting event processor...");

    loop {
        tokio::select! {
            Some(event) = kalshi_rx.recv() => {
                handle_kalshi_event(event);
            }
            // Some(price) = binance_rx.recv() => {
            //     handle_binance_price(price);
            // }
            else => {
                warn!("All channels closed, stopping event processor");
                break;
            }
        }
    }
}

fn handle_kalshi_event(event: KalshiEvent) {
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
            info!(
                "📚 Kalshi {} orderbook | {} yes_bids, {} yes_asks",
                ob.market_ticker,
                ob.yes_bids.len(),
                ob.yes_asks.len()
            );
        }
        KalshiEvent::Trade(trade) => {
            info!(
                "💰 Kalshi {} trade | yes: {:?}, no: {:?}",
                trade.market_ticker, trade.yes_price, trade.no_price
            );
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

