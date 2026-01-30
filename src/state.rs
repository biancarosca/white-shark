use dashmap::DashMap;

use crate::exchanges::kalshi::{KalshiMarket, KalshiOrderbook, KalshiTicker};

#[derive(Clone)]
pub struct KalshiState {
    pub tracked_markets: DashMap<String, KalshiMarket>,
    pub orderbooks: DashMap<String, KalshiOrderbook>,
    pub tickers: DashMap<String, KalshiTicker>,
}

impl KalshiState {
    pub fn new() -> Self {
        Self {
            tracked_markets: DashMap::new(),
            orderbooks: DashMap::new(),
            tickers: DashMap::new(),
        }
    }

    pub fn get_top_bid(&self, market_ticker: &str) -> Option<f64> {
        self.orderbooks
            .get(market_ticker)?
            .yes_bids
            .first()
            .map(|level| level.price)
    }

    pub fn get_top_ask(&self, market_ticker: &str) -> Option<f64> {
        self.orderbooks
            .get(market_ticker)?
            .yes_asks
            .first()
            .map(|level| level.price)
    }

    pub fn get_orderbook(&self, market_ticker: &str) -> Option<KalshiOrderbook> {
        self.orderbooks.get(market_ticker).map(|entry| entry.value().clone())
    }
}

impl Default for KalshiState {
    fn default() -> Self {
        Self::new()
    }
}

