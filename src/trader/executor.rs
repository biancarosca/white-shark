use std::collections::HashSet;
use std::sync::Arc;

use tracing::info;

use super::main::OrderDecision;
use super::positions::{FillStatus, PositionManager};
use crate::error::Result;
use crate::exchanges::kalshi::{OrderSide, OrderType};
use crate::exchanges::kalshi::api::KalshiApi;
use crate::exchanges::kalshi::models::OrderAction;

pub struct OrderExecutor {
    api: Arc<KalshiApi>,
    positions: PositionManager,
}

impl OrderExecutor {
    pub fn new(api: Arc<KalshiApi>, positions: PositionManager) -> Self {
        Self { api, positions }
    }

    pub async fn execute(&self, decision: OrderDecision) -> Result<()> {
        match decision {
            OrderDecision::CancelAll => self.cancel_all().await,
            OrderDecision::Place {
                ref ticker,
                side,
                price,
                contracts,
                order_type,
            } => self.place_order(ticker, side, price, contracts, order_type).await,
        }
    }

    async fn place_order(
        &self,
        ticker: &str,
        side: OrderSide,
        price: f64,
        contracts: u64,
        order_type: OrderType,
    ) -> Result<()> {
        let price_cents = (price * 100.0) as u64;

        info!(
            "Executing {:?} order: {} {:?} {}x @ {}c",
            order_type, ticker, side, contracts, price_cents
        );

        let resp = self
            .api
            .create_order(
                ticker,
                OrderAction::Buy,
                side,
                contracts,
                price_cents,
                order_type,
            )
            .await?;

        let order = &resp.order;
        let status = if order.remaining_count > 0 {
            FillStatus::Open
        } else {
            FillStatus::Filled
        };

        if order.fill_count > 0 || order.remaining_count > 0 {
            info!(
                "Order {}: filled={}, remaining={}",
                order.order_id, order.fill_count, order.remaining_count
            );
            self.positions.add_fill(
                ticker,
                side,
                order.order_id.clone(),
                order.fill_count as u64,
                price,
                status,
            );
        }

        Ok(())
    }

    async fn cancel_all(&self) -> Result<()> {
        let local_open = self.positions.open_order_ids();
        if local_open.is_empty() {
            info!("No open orders tracked locally");
            return Ok(());
        }

        let resting = self.api.get_orders(None, Some("resting")).await?;
        let resting_ids: HashSet<&str> = resting
            .iter()
            .map(|o| o.order_id.as_str())
            .collect();

        let to_cancel: Vec<&str> = local_open
            .iter()
            .filter(|id| resting_ids.contains(id.as_str()))
            .map(|id| id.as_str())
            .collect();

        if to_cancel.is_empty() {
            info!("No matching resting orders to cancel");
            return Ok(());
        }

        info!("Batch cancelling {} orders", to_cancel.len());
        let resp = self.api.batch_cancel_orders(&to_cancel).await?;

        for cancelled in &resp.orders {
            self.positions.mark_cancelled(&cancelled.order_id);
            info!(
                "Cancelled order {}: reduced by {}",
                cancelled.order_id, cancelled.reduced_by
            );
        }

        Ok(())
    }
}
