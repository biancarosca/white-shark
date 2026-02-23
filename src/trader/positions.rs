use std::sync::Arc;

use dashmap::{DashMap, mapref::one::Ref};
use tracing::info;

use crate::exchanges::kalshi::models::OrderSide;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillStatus {
    Open,
    Filled,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct FillEntry {
    pub order_id: String,
    pub price: f64,
    pub contracts: u64,
    pub status: FillStatus,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub side: OrderSide,
    pub entries: Vec<FillEntry>,
}

#[derive(Debug, Clone)]
pub struct PositionManager {
    positions: Arc<DashMap<String, Position>>,
}

impl PositionManager {
    pub fn new() -> Self {
        Self {
            positions: Arc::new(DashMap::new()),
        }
    }

    pub fn add_fill(
        &self,
        ticker: &str,
        side: OrderSide,
        order_id: String,
        contracts: u64,
        price: f64,
        status: FillStatus,
    ) {
        let fill = FillEntry {
            order_id,
            price,
            contracts,
            status,
        };
        let mut position = self.positions.entry(ticker.to_string()).or_insert(Position {
            side,
            entries: Vec::new(),
        });
        position.entries.push(fill);
        info!("Added fill to position: {:?}", position);
    }

    pub fn mark_cancelled(&self, order_id: &str) {
        for mut pos in self.positions.iter_mut() {
            for entry in pos.entries.iter_mut() {
                if entry.order_id == order_id {
                    entry.status = FillStatus::Cancelled;
                    return;
                }
            }
        }
    }

    pub fn open_order_ids_for(&self, ticker: &str) -> Vec<String> {
        self.positions
            .get(ticker)
            .map(|pos| {
                pos.entries
                    .iter()
                    .filter(|e| e.status == FillStatus::Open)
                    .map(|e| e.order_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn open_order_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for pos in self.positions.iter() {
            for entry in &pos.entries {
                if entry.status == FillStatus::Open {
                    ids.push(entry.order_id.clone());
                }
            }
        }
        ids
    }

    pub fn cleanup(&self) {
        self.positions.clear();
    }

    pub fn get(&self, ticker: &str) -> Option<Ref<'_, String, Position>> {
        self.positions.get(ticker)
    }
}
