use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::{error, info};

use crate::exchanges::kalshi::api::KalshiApi;
use crate::exchanges::kalshi::models::{KalshiFill, OrderSide};
use crate::exchanges::kalshi::TickUpdate;

use super::constants::*;
use super::executor::OrderExecutor;

#[derive(Debug, Clone)]
struct OpenOrder {
    order_id: String,
    side: OrderSide,
    price: f64,
    contracts: u64,
}

pub struct Trader {
    executor: OrderExecutor,
    current_ticker: Option<String>,
    total_yes_contracts: i64,
    total_no_contracts: i64,
    yes_cost: f64,
    no_cost: f64,
    last_api_call: Instant,
    latest_tick: Option<TickUpdate>,
    anchor: Option<OpenOrder>,
    rebalance: Option<OpenOrder>,
    fill_rx: mpsc::Receiver<KalshiFill>,
    cancelled_anchors: HashMap<String, OpenOrder>,
    cancelled_rebalances: HashMap<String, OpenOrder>,
    last_yes_anchor_price: Option<f64>,
    last_no_anchor_price: Option<f64>,
}

impl Trader {
    pub fn new(api: Arc<KalshiApi>, fill_rx: mpsc::Receiver<KalshiFill>) -> Self {
        Self {
            executor: OrderExecutor::new(api),
            current_ticker: None,
            total_yes_contracts: 0,
            total_no_contracts: 0,
            yes_cost: 0.0,
            no_cost: 0.0,
            last_api_call: Instant::now() - std::time::Duration::from_secs(10),
            latest_tick: None,
            anchor: None,
            rebalance: None,
            fill_rx,
            cancelled_anchors: HashMap::new(),
            cancelled_rebalances: HashMap::new(),
            last_yes_anchor_price: None,
            last_no_anchor_price: None,
        }
    }

    pub fn spawn(api: Arc<KalshiApi>, series: String) -> (mpsc::Sender<TickUpdate>, mpsc::Sender<KalshiFill>) {
        let (tick_tx, tick_rx) = mpsc::channel::<TickUpdate>(TRADING_CHANNEL_BUFFER);
        let (fill_tx, fill_rx) = mpsc::channel::<KalshiFill>(64);
        let trader = Self::new(api, fill_rx);
        info!("🚀 Spawned trader for series: {}", series);
        tokio::spawn(trader.run(tick_rx));
        (tick_tx, fill_tx)
    }

    async fn run(mut self, mut tick_rx: mpsc::Receiver<TickUpdate>) {
        info!("Trading engine started");
        loop {
            tokio::select! {
                biased;
                Some(fill) = self.fill_rx.recv() => self.on_fill(fill).await,
                Some(mut tick) = tick_rx.recv() => {
                    while let Ok(newer) = tick_rx.try_recv() {
                        tick = newer;
                    }
                    self.on_tick(tick).await;
                },
                else => break,
            }
        }
        info!("Trading engine shutting down");
    }

    fn imbalance(&self) -> i64 {
        self.total_yes_contracts - self.total_no_contracts
    }

    fn throttled(&self) -> bool {
        (self.last_api_call.elapsed().as_millis() as u64) < THROTTLE_MS
    }

    fn bid_for_side(tick: &TickUpdate, side: OrderSide) -> f64 {
        match side {
            OrderSide::Yes => tick.yes_bid,
            OrderSide::No => tick.no_bid,
        }
    }

    async fn cancel_anchor(&mut self) {
        if let Some(anc) = self.anchor.take() {
            self.executor.cancel_order(&anc.order_id).await;
            self.last_api_call = Instant::now();
            self.cancelled_anchors.insert(anc.order_id.clone(), anc);
        }
    }

    async fn cancel_rebalance(&mut self) {
        if let Some(reb) = self.rebalance.take() {
            self.executor.cancel_order(&reb.order_id).await;
            self.last_api_call = Instant::now();
            self.cancelled_rebalances.insert(reb.order_id.clone(), reb);
        }
    }

    fn deficit_side(imbalance: i64) -> OrderSide {
        if imbalance > 0 { OrderSide::No } else { OrderSide::Yes }
    }

    fn rebalance_cap(&self) -> f64 {
        let deficit_needed = self.imbalance().unsigned_abs() as f64;
        if deficit_needed == 0.0 {
            return f64::MAX;
        }
        let n_surplus = self.total_yes_contracts.max(self.total_no_contracts) as f64;
        let budget = MAX_BID_SUM * n_surplus - (self.yes_cost + self.no_cost);
        budget / deficit_needed
    }

    fn record_fill_cost(&mut self, side: OrderSide, price: f64, qty: u64) {
        match side {
            OrderSide::Yes => self.yes_cost += price * qty as f64,
            OrderSide::No => self.no_cost += price * qty as f64,
        }
    }

    fn log_pair_cost(&self) {
        let pairs = self.total_yes_contracts.min(self.total_no_contracts);
        if pairs > 0 {
            let avg_yes = self.yes_cost / self.total_yes_contracts.max(1) as f64;
            let avg_no = self.no_cost / self.total_no_contracts.max(1) as f64;
            let pair_cost = avg_yes + avg_no;
            let guaranteed = pairs as f64 * (1.0 - pair_cost);
            info!("💰 Avg YES={:.0}c NO={:.0}c, pair={:.0}c, pairs={}, guaranteed=${:.2}",
                avg_yes * 100.0, avg_no * 100.0, pair_cost * 100.0, pairs, guaranteed);
        }
    }

    async fn on_tick(&mut self, tick: TickUpdate) {
        if self.current_ticker.as_ref() != Some(&tick.ticker) {
            self.reset_for_new_market(&tick.ticker).await;
        }
        self.latest_tick = Some(tick.clone());

        let secs_to_close = tick.seconds_until_close();

        if secs_to_close.map_or(false, |s| s <= CANCEL_BEFORE_CLOSE_SECS) {
            self.cancel_all().await;
            return;
        }
        
        if self.throttled() {
            return;
        }

        let imb = self.imbalance();
        let abs_imb = imb.unsigned_abs() as u64;

        if abs_imb > 0 {
            let deficit = Self::deficit_side(imb);

            let cap = self.rebalance_cap();
            let market_bid = Self::bid_for_side(&tick, deficit);
            let target_price = market_bid.min(cap);

            let should_cancel_reb = if let Some(ref reb) = self.rebalance {
                let needs_resize = reb.contracts != abs_imb;
                let needs_requote = (target_price - reb.price).abs() >= REQUOTE_THRESHOLD;
                needs_resize || needs_requote
            } else {
                false
            };

            if should_cancel_reb {
                self.cancel_rebalance().await;
            }

            if self.rebalance.is_none() && !self.throttled() {
                if cap < market_bid {
                    info!("⚖️ Rebalance cap: market={:.0}c, cap={:.0}c",
                        market_bid * 100.0, cap * 100.0);
                }
                self.place_rebalance(&tick.ticker, deficit, target_price, abs_imb).await;
            }
        } else if self.rebalance.is_some() {
            self.cancel_rebalance().await;
        }

        if self.throttled() {
            return;
        }

        // Only anchor when balanced — no averaging down
        if abs_imb > 0 {
            self.cancel_anchor().await;
            return;
        }

        let yes_bid = tick.yes_bid;
        let no_bid = tick.no_bid;
        let bid_sum = yes_bid + no_bid;
        let anchor_side = if yes_bid >= no_bid { OrderSide::Yes } else { OrderSide::No };

        if let Some(ref anc) = self.anchor {
            let current_bid = Self::bid_for_side(&tick, anc.side);
            if (current_bid - anc.price).abs() >= REQUOTE_THRESHOLD {
                self.cancel_anchor().await;
            }
        }

        if self.anchor.is_none() && !self.throttled() {
            if bid_sum > MAX_BID_SUM || yes_bid <= 0.0 || no_bid <= 0.0 {
                return;
            }

            let price = Self::bid_for_side(&tick, anchor_side);
            if price < MIN_ANCHOR_PRICE {
                return;
            }

            self.place_anchor(&tick.ticker, anchor_side, price, INITIAL_CONTRACTS).await;
        }
    }

    async fn on_fill(&mut self, fill: KalshiFill) {
        if self.current_ticker.as_deref() != Some(&fill.market_ticker) {
            return;
        }

        let fill_side = if fill.side == "yes" { OrderSide::Yes } else { OrderSide::No };
        let fill_count = fill.get_count();
        match fill_side {
            OrderSide::Yes => self.total_yes_contracts += fill_count,
            OrderSide::No => self.total_no_contracts += fill_count,
        }

        info!(
            "📊 Position: YES={}, NO={}, imbalance={}",
            self.total_yes_contracts, self.total_no_contracts, self.imbalance()
        );

        let is_anchor = self.anchor.as_ref().map_or(false, |a| a.order_id == fill.order_id);
        let is_rebalance = self.rebalance.as_ref().map_or(false, |r| r.order_id == fill.order_id);
        let is_cancelled_anchor = self.cancelled_anchors.contains_key(&fill.order_id);
        let is_cancelled_rebalance = self.cancelled_rebalances.contains_key(&fill.order_id);

        let filled = fill_count as u64;

        if is_anchor || is_cancelled_anchor {
            let price = if is_anchor {
                self.anchor.as_ref().unwrap().price
            } else {
                self.cancelled_anchors.get(&fill.order_id).unwrap().price
            };

            self.record_fill_cost(fill_side, price, filled);
            match fill_side {
                OrderSide::Yes => self.last_yes_anchor_price = Some(price),
                OrderSide::No => self.last_no_anchor_price = Some(price),
            }
            info!("🎣 Anchor filled{}: {:?} x{} @ {}c",
                if is_cancelled_anchor { " (late)" } else { "" },
                fill_side, fill_count, (price * 100.0).round());
            self.log_pair_cost();

            if is_anchor {
                if let Some(ref mut anc) = self.anchor {
                    anc.contracts = anc.contracts.saturating_sub(filled);
                    info!("🎣 Anchor remaining: {}", anc.contracts);
                    if anc.contracts == 0 {
                        self.anchor = None;
                    }
                }
            } else {
                self.cancelled_anchors.remove(&fill.order_id);
            }

            if let Some(ref reb) = self.rebalance {
                info!("Cancelling stale rebalance {} (imbalance changed)", reb.order_id);
            }
            self.cancel_rebalance().await;
        } else if is_rebalance || is_cancelled_rebalance {
            let price = if is_rebalance {
                self.rebalance.as_ref().unwrap().price
            } else {
                self.cancelled_rebalances.get(&fill.order_id).unwrap().price
            };

            self.record_fill_cost(fill_side, price, filled);
            info!("⚖️ Rebalance filled{}: {:?} x{} @ {}c",
                if is_cancelled_rebalance { " (late)" } else { "" },
                fill_side, fill_count, (price * 100.0).round());

            if is_rebalance {
                if let Some(ref mut reb) = self.rebalance {
                    reb.contracts = reb.contracts.saturating_sub(filled);
                    info!("⚖️ Rebalance remaining: {}", reb.contracts);
                    if reb.contracts == 0 {
                        self.rebalance = None;
                    }
                }
            } else {
                let remove = {
                    let entry = self.cancelled_rebalances.get_mut(&fill.order_id).unwrap();
                    entry.contracts = entry.contracts.saturating_sub(filled);
                    entry.contracts == 0
                };
                if remove {
                    self.cancelled_rebalances.remove(&fill.order_id);
                }
            }
            self.log_pair_cost();

            if self.imbalance() == 0 {
                self.last_yes_anchor_price = None;
                self.last_no_anchor_price = None;
            }
        } else {
            info!("Fill for order {} not tracked, ignoring", fill.order_id);
        }
    }

    async fn place_anchor(&mut self, ticker: &str, side: OrderSide, price: f64, contracts: u64) {
        let price_cents = (price * 100.0).round() as u64;
        info!("🎣 Placing anchor: {:?} {} @ {}c x{}", side, ticker, price_cents, contracts);

        match self.executor.place_order(ticker, side, price_cents, contracts).await {
            Ok(result) => {
                self.last_api_call = Instant::now();
                if result.fill_count > 0 {
                    self.record_fill_cost(side, price, result.fill_count as u64);
                    match side {
                        OrderSide::Yes => self.total_yes_contracts += result.fill_count,
                        OrderSide::No => self.total_no_contracts += result.fill_count,
                    }
                }
                if result.remaining_count > 0 {
                    self.anchor = Some(OpenOrder {
                        order_id: result.order_id,
                        side,
                        price,
                        contracts: result.remaining_count as u64,
                    });
                }
            }
            Err(e) => {
                error!("Failed to place anchor: {}", e);
                self.last_api_call = Instant::now();
            }
        }
    }

    async fn place_rebalance(&mut self, ticker: &str, side: OrderSide, price: f64, contracts: u64) {
        let price_cents = (price * 100.0).round() as u64;
        info!("⚖️ Placing rebalance: {:?} {} @ {}c x{}", side, ticker, price_cents, contracts);

        match self.executor.place_order(ticker, side, price_cents, contracts).await {
            Ok(result) => {
                self.last_api_call = Instant::now();
                if result.fill_count > 0 {
                    self.record_fill_cost(side, price, result.fill_count as u64);
                    match side {
                        OrderSide::Yes => self.total_yes_contracts += result.fill_count,
                        OrderSide::No => self.total_no_contracts += result.fill_count,
                    }
                }
                if result.remaining_count > 0 {
                    self.rebalance = Some(OpenOrder {
                        order_id: result.order_id,
                        side,
                        price,
                        contracts: result.remaining_count as u64,
                    });
                }
            }
            Err(e) => {
                error!("Failed to place rebalance: {}", e);
                self.last_api_call = Instant::now();
            }
        }
    }

    async fn cancel_all(&mut self) {
        self.cancel_anchor().await;
        self.cancel_rebalance().await;
    }

    async fn reset_for_new_market(&mut self, new_ticker: &str) {
        if self.current_ticker.is_some() {
            info!("🔄 Market rotated to {}, resetting trader", new_ticker);
            self.cancel_all().await;
        }
        self.current_ticker = Some(new_ticker.to_string());
        self.total_yes_contracts = 0;
        self.total_no_contracts = 0;
        self.yes_cost = 0.0;
        self.no_cost = 0.0;
        self.cancelled_anchors.clear();
        self.cancelled_rebalances.clear();
        self.last_yes_anchor_price = None;
        self.last_no_anchor_price = None;
    }
}
