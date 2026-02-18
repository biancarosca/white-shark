use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info};

use super::constants::{BATCH_SIZE, CHANNEL_BUFFER_SIZE, FLUSH_INTERVAL_MS};
use super::models::KalshiOrderbook;
use crate::db::main::Db;

#[derive(Clone)]
pub struct MarketDataUpdate {
    pub ticker: String,
    pub asset: String,
    pub timestamp: DateTime<Utc>,
    pub yes_ask: f64,
    pub yes_bid: f64,
    pub no_ask: f64,
    pub no_bid: f64,
}

impl MarketDataUpdate {
    pub fn from_orderbook(ob: &KalshiOrderbook, asset: String) -> Self {
        Self {
            ticker: ob.market_ticker.clone(),
            asset,
            timestamp: Utc::now(),
            yes_ask: ob.top_yes_ask(),
            yes_bid: ob.top_yes_bid(),
            no_ask: ob.top_no_ask(),
            no_bid: ob.top_no_bid(),
        }
    }
}

pub struct MarketDataWriter;

impl MarketDataWriter {
    pub fn spawn(db: Arc<Db>) -> mpsc::Sender<MarketDataUpdate> {
        let (tx, rx) = mpsc::channel::<MarketDataUpdate>(CHANNEL_BUFFER_SIZE);
        tokio::spawn(Self::run(db, rx));
        tx
    }

    async fn run(db: Arc<Db>, mut rx: mpsc::Receiver<MarketDataUpdate>) {
        let mut batch: Vec<MarketDataUpdate> = Vec::with_capacity(BATCH_SIZE);
        let mut flush_interval = interval(Duration::from_millis(FLUSH_INTERVAL_MS));

        loop {
            tokio::select! {
                maybe_update = rx.recv() => {
                    match maybe_update {
                        Some(update) => {
                            batch.push(update);
                            if batch.len() >= BATCH_SIZE {
                                Self::flush(&db, &mut batch).await;
                            }
                        }
                        None => {
                            if !batch.is_empty() {
                                Self::flush(&db, &mut batch).await;
                            }
                            info!("Market data writer shutting down");
                            break;
                        }
                    }
                }
                _ = flush_interval.tick() => {
                    if !batch.is_empty() {
                        Self::flush(&db, &mut batch).await;
                    }
                }
            }
        }
    }

    async fn flush(db: &Db, batch: &mut Vec<MarketDataUpdate>) {
        if batch.is_empty() {
            return;
        }

        let count = batch.len();
        let records: Vec<_> = batch
            .drain(..)
            .map(|u| (u.ticker, u.asset, u.timestamp, u.yes_ask, u.yes_bid, u.no_ask, u.no_bid))
            .collect();

        if let Err(e) = db.insert_market_data_batch(records).await {
            error!("Failed to batch insert market data: {}", e);
        } else {
            info!("📝 Flushed {} market data records to DB", count);
        }
    }
}
