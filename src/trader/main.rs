use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::exchanges::kalshi::api::KalshiApi;
use crate::exchanges::kalshi::models::{OrderSide, OrderType};
use crate::exchanges::kalshi::TickUpdate;
use crate::utils::trade::get_contract_size;

use super::constants::{
    CANCEL_BEFORE_CLOSE_SECS, EXIT_ASK_THRESHOLD, FILL_OR_KILL_ORDER_PRICE, LADDER_PRICES,
    LEVEL_1_CONTRACTS, ORDER_COOLDOWN_SECS, TRADING_CHANNEL_BUFFER,
};
use super::executor::OrderExecutor;
use super::positions::PositionManager;

#[derive(Debug)]
pub enum OrderDecision {
    Place {
        ticker: String,
        side: OrderSide,
        price: f64,
        contracts: u64,
        order_type: OrderType,
    },
    CancelAll,
}

pub struct Trader {
    positions: PositionManager,
    executor: OrderExecutor,
    latest_ticks: HashMap<String, TickUpdate>,
    laddered_tickers: HashSet<String>,
    cooldowns: HashMap<String, Instant>,
    should_exit: bool,
}

impl Trader {
    pub fn new(api: Arc<KalshiApi>) -> Self {
        let positions = PositionManager::new();
        let executor = OrderExecutor::new(api, positions.clone());
        Self {
            positions,
            executor,
            latest_ticks: HashMap::new(),
            laddered_tickers: HashSet::new(),
            cooldowns: HashMap::new(),
            should_exit: false,
        }
    }

    pub fn spawn(api: Arc<KalshiApi>) -> mpsc::Sender<TickUpdate> {
        let (tx, rx) = mpsc::channel::<TickUpdate>(TRADING_CHANNEL_BUFFER);
        let trader = Self::new(api);
        tokio::spawn(trader.run(rx));
        tx
    }

    async fn run(mut self, mut rx: mpsc::Receiver<TickUpdate>) {
        info!("Trading engine started");
        while let Some(tick) = rx.recv().await {
            self.on_tick(&tick).await;
        }
        info!("Trading engine shutting down");
    }

    async fn on_tick(&mut self, tick: &TickUpdate) {
        if self.should_exit {
            if !self.latest_ticks.contains_key(&tick.ticker) {
                self.cleanup();
                self.should_exit = false;
            } else {
                return;
            }
        }

        self.latest_ticks
            .insert(tick.ticker.clone(), tick.clone());

        let decisions = self.decide(tick);
        for (i, decision) in decisions.into_iter().enumerate() {
            if i > 0 {
                if let OrderDecision::Place { .. } = &decision {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
            }

            let ticker = match &decision {
                OrderDecision::CancelAll => {
                    self.should_exit = true;
                    None
                }
                OrderDecision::Place { ticker, .. } => Some(ticker.clone()),
            };
            
            info!("Decision: {:?}", decision);
            if let Err(e) = self.executor.execute(decision).await {
                error!("Order execution failed: {}", e);
                if let Some(t) = ticker {
                    warn!("Cooling down ticker {} for {}s after failure", t, ORDER_COOLDOWN_SECS);
                    self.cooldowns.insert(t, Instant::now());
                }
                continue;
            }
        }
    }

    fn decide(&mut self, tick: &TickUpdate) -> Vec<OrderDecision> {
        let is_near_close = tick
            .seconds_until_close()
            .map_or(false, |s| s <= CANCEL_BEFORE_CLOSE_SECS);

        if is_near_close {
            if self.positions.get(&tick.ticker).is_some() {
                info!("Market closing soon, cancelling orders");
                return vec![OrderDecision::CancelAll];
            }
        }

        if self.all_asks_below_threshold() {
            info!("All asks below exit threshold, cancelling all orders");
            return vec![OrderDecision::CancelAll];
        }

        if let Some(pos) = self.positions.get(&tick.ticker) {
            if !self.laddered_tickers.contains(&tick.ticker) {
                let side = pos.side;
                drop(pos);
                self.laddered_tickers.insert(tick.ticker.clone());
                return self.build_ladder(&tick.ticker, side);
            }
            return vec![];
        }

        if self.is_on_cooldown(&tick.ticker) {
            return vec![];
        }

        if let Some(side) = self.entry_side(tick) {
            return vec![OrderDecision::Place {
                ticker: tick.ticker.clone(),
                side,
                price: FILL_OR_KILL_ORDER_PRICE,
                contracts: LEVEL_1_CONTRACTS,
                order_type: OrderType::Market,
            }];
        }

        vec![]
    }

    fn cleanup(&mut self) {
        self.latest_ticks.clear();
        self.laddered_tickers.clear();
        self.cooldowns.clear();
        self.positions.cleanup();
        info!("Trader cleaned up");
    }

    fn is_on_cooldown(&self, ticker: &str) -> bool {
        if let Some(since) = self.cooldowns.get(ticker) {
            since.elapsed().as_secs() < ORDER_COOLDOWN_SECS
        } else {
            false
        }
    }

    fn entry_side(&self, tick: &TickUpdate) -> Option<OrderSide> {
        let min_qty = LEVEL_1_CONTRACTS as i64;
        if tick.yes_ask >= 0.99 && tick.yes_bid >= 0.98 && tick.yes_ask_qty >= min_qty {
            return Some(OrderSide::Yes);
        }
        if tick.no_ask >= 0.99 && tick.no_bid >= 0.98 && tick.no_ask_qty >= min_qty {
            return Some(OrderSide::No);
        }
        None
    }

    fn all_asks_below_threshold(&self) -> bool {
        if self.latest_ticks.is_empty() {
            return false;
        }

        self.latest_ticks.values().all(|tick| {
            let ask = match self.positions.get(&tick.ticker) {
                Some(pos) => match pos.side {
                    OrderSide::Yes => tick.yes_ask,
                    OrderSide::No => tick.no_ask,
                },
                None => 0.0,
            };
            if !(ask > 0.0) {
                return false;
            }
            ask <= EXIT_ASK_THRESHOLD
        })
    }

    fn build_ladder(&self, ticker: &str, side: OrderSide) -> Vec<OrderDecision> {
        LADDER_PRICES
            .iter()
            .map(|&price| OrderDecision::Place {
                ticker: ticker.to_string(),
                side,
                price,
                contracts: get_contract_size(price),
                order_type: OrderType::Limit,
            })
            .collect()
    }
}
