use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info};

use super::constants::{BATCH_SIZE, CHANNEL_BUFFER_SIZE, FLUSH_INTERVAL_MS};
use crate::db::main::Db;
use crate::exchanges::kalshi::TickUpdate;

pub struct MarketDataWriter;

impl MarketDataWriter {
    pub fn spawn(db: Arc<Db>) -> mpsc::Sender<TickUpdate> {
        let (tx, rx) = mpsc::channel::<TickUpdate>(CHANNEL_BUFFER_SIZE);
        tokio::spawn(Self::run(db, rx));
        tx
    }

    async fn run(db: Arc<Db>, mut rx: mpsc::Receiver<TickUpdate>) {
        let mut batch: Vec<TickUpdate> = Vec::with_capacity(BATCH_SIZE);
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

    async fn flush(db: &Db, batch: &mut Vec<TickUpdate>) {
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
