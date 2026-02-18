use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{error, info, warn};

use super::api::KalshiApi;
use super::context::ClientContext;
use super::models::KalshiMarketStatus;
use super::websocket::KalshiWebSocket;
use crate::error::{Error, Result};
use crate::exchanges::kalshi::constants::*;

pub(crate) struct SubscriptionManager;

impl SubscriptionManager {
    pub async fn subscribe_all(
        ctx: &mut ClientContext,
        ws: &Arc<Mutex<KalshiWebSocket>>,
    ) -> Result<()> {
        if ctx.current_markets.is_empty() {
            return Err(Error::Other("No current markets set".into()));
        }

        let tickers: Vec<String> = ctx.current_markets.values().map(|m| m.ticker.clone()).collect();
        let mut ws_guard = ws.lock().await;

        if let Some(sid) = ctx.subscription_ids.remove("orderbook_delta") {
            info!("⛓️‍💥 Unsubscribing from orderbook with sid: {:?}", sid);
            ws_guard.unsubscribe(vec![sid]).await?;
        }

        info!("📡 Subscribing to {} markets: {:?}", tickers.len(), tickers);
        ws_guard.subscribe_orderbook(tickers).await?;

        if !ctx.subscription_ids.contains_key("market_lifecycle_v2") {
            info!("🤝 Subscribing to market lifecycle");
            ws_guard.subscribe_market_lifecycle().await?;
        }

        Ok(())
    }

    pub async fn fetch_and_set_all(ctx: &mut ClientContext, api: &KalshiApi) -> Result<()> {
        for series_ticker in &ctx.series_tickers.clone() {
            if let Some(existing) = ctx.current_markets.get(series_ticker) {
                if matches!(existing.status, KalshiMarketStatus::Open | KalshiMarketStatus::Active) {
                    continue;
                }
            }

            let markets = api.fetch_market_by_ticker(series_ticker, Some("open")).await?;

            if markets.is_empty() {
                warn!("No open markets found for series: {}", series_ticker);
                ctx.current_markets.remove(series_ticker);
                continue;
            }

            let next_market = &markets[0];

            if !matches!(next_market.status, KalshiMarketStatus::Open | KalshiMarketStatus::Active) {
                info!("Fetched market {} is {:?}, skipping", next_market.ticker, next_market.status);
                ctx.current_markets.remove(series_ticker);
                continue;
            }

            if let Some(old_market) = ctx.current_markets.get(series_ticker) {
                if old_market.ticker == next_market.ticker {
                    info!("API returned same market {}, skipping", next_market.ticker);
                    ctx.current_markets.remove(series_ticker);
                    continue;
                }
                info!(
                    "🔄 Replacing market {} with {} for series {}",
                    old_market.ticker, next_market.ticker, series_ticker
                );
            } else {
                info!("📡 Setting initial market for {}: {}", series_ticker, next_market.ticker);
            }

            ctx.current_markets.insert(series_ticker.clone(), next_market.clone());
            ctx.market_to_series.insert(next_market.ticker.clone(), series_ticker.clone());
            ctx.track_market(next_market);

            if let Some(floor_strike) = next_market.extra.get("floor_strike") {
                info!("💰 Floor strike for {}: {}", next_market.ticker, floor_strike);
            }
        }

        Ok(())
    }

    pub async fn handle_due_markets(
        ctx: &mut ClientContext,
        api: &KalshiApi,
        ws: &Arc<Mutex<KalshiWebSocket>>,
    ) -> Result<()> {
        info!("⏰ 15-minute interval reached, rotating all markets...");
        ctx.current_markets.clear();

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Err(e) = Self::fetch_and_set_all(ctx, api).await {
                error!("Failed to fetch all markets (attempt {}): {}", attempt, e);
                if attempt >= MAX_MARKET_FETCH_ATTEMPTS {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_secs(MARKET_FETCH_INTERVAL_SECS)).await;
                continue;
            }

            let all_open = ctx.series_tickers.iter().all(|st| {
                ctx.current_markets
                    .get(st)
                    .map(|m| matches!(m.status, KalshiMarketStatus::Open | KalshiMarketStatus::Active))
                    .unwrap_or(false)
            });

            if all_open {
                break;
            }

            if attempt >= MAX_MARKET_FETCH_ATTEMPTS {
                warn!(
                    "Gave up after {} attempts, some markets still not open",
                    MAX_MARKET_FETCH_ATTEMPTS
                );
                break;
            }

            info!(
                "Not all markets open yet, retrying in {}s (attempt {}/{})",
                MARKET_FETCH_INTERVAL_SECS, attempt, MAX_MARKET_FETCH_ATTEMPTS
            );
            tokio::time::sleep(Duration::from_secs(MARKET_FETCH_INTERVAL_SECS)).await;
        }

        Self::subscribe_all(ctx, ws).await?;
        Ok(())
    }
}
