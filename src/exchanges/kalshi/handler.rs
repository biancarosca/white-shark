use chrono::Utc;
use tracing::{error, info, warn};

use super::context::ClientContext;
use super::models::{
    KalshiMarketLifecycleMsg, KalshiMarketStatus, KalshiOrderbook, KalshiOrderbookDelta,
    KalshiOrderbookSnapshot, KalshiWsMessage,
};
use crate::error::Result;

pub(crate) struct MessageHandler;

impl MessageHandler {
    pub async fn handle(ctx: &mut ClientContext, msg: KalshiWsMessage) -> Result<()> {
        if msg.is_subscribed() {
            return Self::on_subscription_confirm(ctx, &msg);
        }

        if let Some(err) = &msg.error {
            error!("WebSocket error: {}", err);
            return Ok(());
        }

        if msg.msg_type.as_deref() == Some("error") {
            if let Some(payload) = msg.payload() {
                error!("WebSocket error message: {:?}", payload);
            }
            return Ok(());
        }

        let payload = match msg.payload() {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        match msg.msg_type.as_deref() {
            Some("orderbook_snapshot") => Self::on_orderbook_snapshot(ctx, payload).await,
            Some("orderbook_delta") => Self::on_orderbook_delta(ctx, payload).await,
            Some("market_lifecycle_v2") => Self::on_market_lifecycle(ctx, payload).await,
            _ => Ok(()),
        }
    }

    fn on_subscription_confirm(ctx: &mut ClientContext, msg: &KalshiWsMessage) -> Result<()> {
        let sid = msg.payload().and_then(|p| p.get("sid")).and_then(|s| s.as_u64());
        let channel = msg
            .payload()
            .and_then(|p| p.get("channel"))
            .and_then(|c| c.as_str());

        match (sid, channel) {
            (Some(sid), Some(ch)) => {
                info!("✅ Subscription confirmed: sid={}, channel={}", sid, ch);
                ctx.subscription_ids.insert(ch.to_string(), sid);
            }
            _ => {
                error!(
                    "Incomplete subscription confirmation: sid={:?}, channel={:?}",
                    sid, channel
                );
            }
        }
        Ok(())
    }

    async fn on_orderbook_snapshot(ctx: &ClientContext, payload: serde_json::Value) -> Result<()> {
        let snapshot: KalshiOrderbookSnapshot = match serde_json::from_value(payload.clone()) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to parse orderbook snapshot: {}, payload: {:?}", e, payload);
                return Ok(());
            }
        };

        let ticker = snapshot.market_ticker.clone();
        let mut entry = ctx
            .state
            .orderbooks
            .entry(ticker.clone())
            .or_insert_with(|| KalshiOrderbook::new_empty(ticker));

        entry.apply_snapshot(snapshot);
        entry.log_summary();
        ctx.queue_market_data_update(&entry);

        Ok(())
    }

    async fn on_orderbook_delta(ctx: &ClientContext, payload: serde_json::Value) -> Result<()> {
        let delta: KalshiOrderbookDelta = match serde_json::from_value(payload.clone()) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to parse orderbook delta: {}, payload: {:?}", e, payload);
                return Ok(());
            }
        };

        let mut entry = ctx
            .state
            .orderbooks
            .entry(delta.market_ticker.clone())
            .or_insert_with(|| KalshiOrderbook::new_empty(delta.market_ticker.clone()));

        if let Err(e) = entry.apply_delta(&delta) {
            warn!("{}", e);
            return Ok(());
        }

        entry.log_summary();
        ctx.queue_market_data_update(&entry);

        Ok(())
    }

    async fn on_market_lifecycle(ctx: &mut ClientContext, payload: serde_json::Value) -> Result<()> {
        let msg: KalshiMarketLifecycleMsg = match serde_json::from_value(payload.clone()) {
            Ok(m) => m,
            Err(e) => {
                warn!("Failed to parse market lifecycle: {}, payload: {:?}", e, payload);
                return Ok(());
            }
        };

        let series_ticker = match ctx.resolve_series_ticker(&msg.market_ticker) {
            Some(s) => s,
            None => return Ok(()),
        };

        info!(
            "📊 Market lifecycle for {} (series {}): {:?}",
            msg.market_ticker, series_ticker, msg
        );

        let new_status = match msg.event_type.to_status(msg.is_deactivated) {
            Some(status) => status,
            None => {
                warn!("Failed to map event_type to status: {:?}", msg.event_type);
                return Ok(());
            }
        };

        info!(
            "🔄 {} -> {:?} (event: {:?})",
            msg.market_ticker, new_status, msg.event_type
        );

        if new_status == KalshiMarketStatus::Closed || new_status == KalshiMarketStatus::Settled {
            Self::on_market_close(ctx, &msg, &series_ticker).await;
        }

        Ok(())
    }

    async fn on_market_close(
        ctx: &mut ClientContext,
        msg: &KalshiMarketLifecycleMsg,
        series_ticker: &str,
    ) {
        if let Some(result) = &msg.result {
            let strike_price = ctx
                .current_markets
                .get(series_ticker)
                .and_then(|m| m.extra.get("floor_strike"))
                .and_then(|v| v.as_f64())
                .or_else(|| {
                    let tracked = ctx.state.tracked_markets.get(&msg.market_ticker)?;
                    tracked.extra.get("floor_strike")?.as_f64()
                });

            if let Err(e) = ctx
                .db
                .insert_market_info(&msg.market_ticker, Utc::now(), strike_price, result)
                .await
            {
                error!("Failed to insert market info: {}", e);
            }
        }

        info!(
            "🔴 Market {} closed, unsubscribing for series {}...",
            msg.market_ticker, series_ticker
        );

        ctx.market_to_series.remove(&msg.market_ticker);

        let is_still_current = ctx
            .current_markets
            .get(series_ticker)
            .map(|m| m.ticker == msg.market_ticker)
            .unwrap_or(false);
        if is_still_current {
            ctx.current_markets.remove(series_ticker);
        }
    }
}
