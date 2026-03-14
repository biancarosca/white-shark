use std::sync::Arc;

use tracing::{error, info};

use crate::error::Result;
use crate::exchanges::kalshi::api::KalshiApi;
use crate::exchanges::kalshi::models::{OrderAction, OrderSide, OrderType};

pub struct PlaceOrderResult {
    pub order_id: String,
    pub fill_count: i64,
    pub remaining_count: i64,
}

pub struct OrderExecutor {
    api: Arc<KalshiApi>,
}

impl OrderExecutor {
    pub fn new(api: Arc<KalshiApi>) -> Self {
        Self { api }
    }

    pub async fn place_order(
        &self,
        ticker: &str,
        side: OrderSide,
        price_cents: u64,
        contracts: u64,
    ) -> Result<PlaceOrderResult> {
        info!(
            "📤 Placing order: {:?} {} @ {}c x{}",
            side, ticker, price_cents, contracts
        );

        let resp = self
            .api
            .create_order(
                ticker,
                OrderAction::Buy,
                side,
                contracts,
                price_cents,
                OrderType::Limit,
            )
            .await?;

        let order = &resp.order;
        let fill_count = order.get_fill_count();
        let remaining_count = order.get_remaining_count();
        info!(
            "Order {}: filled={}, remaining={}",
            order.order_id, fill_count, remaining_count
        );

        Ok(PlaceOrderResult {
            order_id: order.order_id.clone(),
            fill_count,
            remaining_count,
        })
    }

    pub async fn cancel_order(&self, order_id: &str) {
        if order_id.is_empty() {
            return;
        }
        info!("❌ Cancelling order {}", order_id);
        if let Err(e) = self.api.batch_cancel_orders(&[order_id]).await {
            error!("Failed to cancel order: {}", e);
        }
    }
}
