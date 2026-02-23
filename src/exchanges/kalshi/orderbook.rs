use chrono::Utc;
use tracing::info;

use super::models::{KalshiOrderbook, KalshiOrderbookDelta, KalshiOrderbookSnapshot, OrderbookLevel};

impl KalshiOrderbook {
    pub fn new_empty(market_ticker: String) -> Self {
        Self {
            market_ticker,
            yes_bids: Vec::new(),
            yes_asks: Vec::new(),
            no_bids: Vec::new(),
            no_asks: Vec::new(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: KalshiOrderbookSnapshot) {
        self.yes_bids = Self::parse_dollar_levels(snapshot.yes_dollars);
        self.no_bids = Self::parse_dollar_levels(snapshot.no_dollars);

        info!(
            "📸 Orderbook snapshot for {} ({} YES levels, {} NO levels)",
            self.market_ticker,
            self.yes_bids.len(),
            self.no_bids.len()
        );

        self.derive_asks_from_bids();
        self.sort();
    }

    pub fn apply_delta(&mut self, delta: &KalshiOrderbookDelta) -> std::result::Result<(), String> {
        let price = delta
            .price_dollars
            .parse::<f64>()
            .map_err(|e| format!("Failed to parse delta price '{}': {}", delta.price_dollars, e))?;

        let levels = if delta.side.eq_ignore_ascii_case("yes") {
            &mut self.yes_bids
        } else {
            &mut self.no_bids
        };

        if let Some(idx) = levels.iter().position(|l| (l.price - price).abs() < 1e-12) {
            let new_qty = levels[idx].quantity.saturating_add(delta.delta);
            if new_qty <= 0 {
                levels.remove(idx);
            } else {
                levels[idx].quantity = new_qty;
            }
        } else if delta.delta > 0 {
            levels.push(OrderbookLevel {
                price,
                quantity: delta.delta,
            });
        }

        self.sort();
        self.derive_asks_from_bids();
        self.sort();

        Ok(())
    }

    pub fn derive_asks_from_bids(&mut self) {
        self.yes_asks = self
            .no_bids
            .iter()
            .map(|bid| OrderbookLevel {
                price: 1.0 - bid.price,
                quantity: bid.quantity,
            })
            .collect();

        self.no_asks = self
            .yes_bids
            .iter()
            .map(|bid| OrderbookLevel {
                price: 1.0 - bid.price,
                quantity: bid.quantity,
            })
            .collect();
    }

    pub fn sort(&mut self) {
        let desc = |a: &OrderbookLevel, b: &OrderbookLevel| {
            b.price
                .partial_cmp(&a.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        let asc = |a: &OrderbookLevel, b: &OrderbookLevel| {
            a.price
                .partial_cmp(&b.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        };

        self.yes_bids.sort_by(desc);
        self.no_bids.sort_by(desc);
        self.yes_asks.sort_by(asc);
        self.no_asks.sort_by(asc);
    }

    pub fn log_summary(&self) {
        let fmt_level = |l: Option<&OrderbookLevel>| {
            l.map(|l| format!("${:.4} @ {}", l.price, l.quantity))
                .unwrap_or_else(|| "N/A".to_string())
        };

        info!(
            "📚 Kalshi {} | YES bid: {} | YES ask: {} | NO bid: {} | NO ask: {} at {}",
            self.market_ticker,
            fmt_level(self.yes_bids.first()),
            fmt_level(self.yes_asks.first()),
            fmt_level(self.no_bids.first()),
            fmt_level(self.no_asks.first()),
            Utc::now()
        );
    }

    pub fn top_yes_bid(&self) -> f64 {
        self.yes_bids.first().map(|l| l.price).unwrap_or(0.0)
    }

    pub fn top_yes_ask(&self) -> f64 {
        self.yes_asks.first().map(|l| l.price).unwrap_or(0.0)
    }

    pub fn top_no_bid(&self) -> f64 {
        self.no_bids.first().map(|l| l.price).unwrap_or(0.0)
    }

    pub fn top_no_ask(&self) -> f64 {
        self.no_asks.first().map(|l| l.price).unwrap_or(0.0)
    }

    pub fn yes_ask_qty_at_or_above(&self, min_price: f64) -> i64 {
        self.yes_asks
            .iter()
            .filter(|l| l.price >= min_price - 1e-12)
            .map(|l| l.quantity)
            .sum()
    }

    pub fn no_ask_qty_at_or_above(&self, min_price: f64) -> i64 {
        self.no_asks
            .iter()
            .filter(|l| l.price >= min_price - 1e-12)
            .map(|l| l.quantity)
            .sum()
    }

    fn parse_dollar_levels(dollars: Vec<(String, i64)>) -> Vec<OrderbookLevel> {
        dollars
            .into_iter()
            .filter_map(|(p, q)| {
                p.parse::<f64>()
                    .ok()
                    .map(|price| OrderbookLevel { price, quantity: q })
            })
            .collect()
    }
}
