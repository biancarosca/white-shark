use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info};

use super::market_data::MarketDataUpdate;
use super::models::{KalshiMarket, KalshiOrderbook};
use crate::db::main::Db;
use crate::state::KalshiState;

pub(crate) struct ClientContext {
    pub state: KalshiState,
    pub current_markets: HashMap<String, KalshiMarket>,
    pub market_to_series: HashMap<String, String>,
    pub series_tickers: Vec<String>,
    pub subscription_ids: HashMap<String, u64>,
    pub db: Arc<Db>,
    pub market_data_tx: mpsc::Sender<MarketDataUpdate>,
}

impl ClientContext {
    pub fn new(
        series_tickers: Vec<String>,
        db: Arc<Db>,
        market_data_tx: mpsc::Sender<MarketDataUpdate>,
    ) -> Self {
        Self {
            state: KalshiState::new(),
            current_markets: HashMap::new(),
            market_to_series: HashMap::new(),
            series_tickers,
            subscription_ids: HashMap::new(),
            db,
            market_data_tx,
        }
    }

    pub fn resolve_series_ticker(&self, market_ticker: &str) -> Option<String> {
        self.market_to_series
            .get(market_ticker)
            .cloned()
            .or_else(|| {
                self.current_markets
                    .iter()
                    .find(|(_, m)| m.ticker == market_ticker)
                    .map(|(series, _)| series.clone())
            })
    }

    pub fn queue_market_data_update(&self, ob: &KalshiOrderbook) {
        let asset = match self.resolve_series_ticker(&ob.market_ticker) {
            Some(s) => s,
            None => {
                error!("No series ticker found for market: {}", ob.market_ticker);
                return;
            }
        };

        let update = MarketDataUpdate::from_orderbook(ob, asset);
        if let Err(e) = self.market_data_tx.try_send(update) {
            error!("Failed to queue market data update: {}", e);
        }
    }

    pub fn track_market(&self, market: &KalshiMarket) {
        info!("🪄 Tracking market: {} ({:?})", market.ticker, market.status);
        self.state
            .tracked_markets
            .insert(market.ticker.clone(), market.clone());
    }
}
