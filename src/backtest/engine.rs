use std::fs::File;
use std::io::{BufRead, BufReader, Write};

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use tracing::info;

use crate::db::main::{Db, MarketDataRow};

#[derive(Debug)]
pub enum TradeSide {
    YES = 1,
    NO = 2,
}

#[derive(Debug)]
pub enum TradeType {
    REBALANCE = 1,
    LADDER = 2,
}

#[derive(Debug)]
pub struct FilledOrder {
    pub price: f64,
    pub contracts: f64,
    pub trade_type: TradeType,
    pub trade_side: TradeSide,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug)]
pub struct OpenOrder {
    pub price: f64,
    pub contracts: f64,
}

pub struct BacktestEngine {
    balance: f64,
    yes_ladder_anchor_price: Option<f64>,
    no_ladder_anchor_price: Option<f64>,
    yes_ladder: Vec<f64>,
    no_ladder: Vec<f64>,
    open_yes_rebalance: Option<OpenOrder>,
    open_no_rebalance: Option<OpenOrder>,
    filled_yes_orders: Vec<FilledOrder>,
    filled_no_orders: Vec<FilledOrder>,
    asset: Option<String>,
    last_yes_ask: Option<f64>,
    last_no_ask: Option<f64>,
}

const STEP: f64 = 0.05;
const INITIAL_WALLET_BALANCE: f64 = 100.0;
const LADDER_LENGTH: usize = 10;
const DOLLAR_PER_ORDER: f64 = 5.0;
const REBALANCE_SUM: f64 = 0.98;

fn round_to_nearest_cent(price: f64) -> f64 {
    (price * 100.0).round() / 100.0
}

fn escape_csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

impl BacktestEngine {
    pub fn new() -> Self {
        Self {
            balance: INITIAL_WALLET_BALANCE,
            yes_ladder_anchor_price: None,
            no_ladder_anchor_price: None,
            yes_ladder: Vec::new(),
            no_ladder: Vec::new(),
            open_yes_rebalance: None,
            open_no_rebalance: None,
            filled_yes_orders: Vec::new(),
            filled_no_orders: Vec::new(),
            asset: None,
            last_yes_ask: None,
            last_no_ask: None
        }
    }

    fn create_ladder(&self, anchor: f64, is_yes: bool) -> Vec<f64> {
        let total_yes: f64 = self.filled_yes_orders.iter().map(|o| o.contracts).sum();
        let total_no: f64 = self.filled_no_orders.iter().map(|o| o.contracts).sum();
        let current_total_spent: f64 = self.filled_yes_orders.iter().map(|o| o.price * o.contracts).sum::<f64>() 
                                     + self.filled_no_orders.iter().map(|o| o.price * o.contracts).sum::<f64>();
    
        (1..=LADDER_LENGTH)
            .map(|i| round_to_nearest_cent(anchor - (i as f64 * STEP)).max(0.01))
            .filter(|&p| {
                let new_qty = DOLLAR_PER_ORDER / p;
                
                let (hypothetical_yes, hypothetical_no) = if is_yes {
                    (total_yes + new_qty, total_no)
                } else {
                    (total_yes, total_no + new_qty)
                };
                
                let imbalance = (hypothetical_yes - hypothetical_no).abs();
                let current_spent = current_total_spent + DOLLAR_PER_ORDER;
                
                let max_hedge_cost = imbalance * (REBALANCE_SUM - p).max(0.10); 
    
                (current_spent + max_hedge_cost + DOLLAR_PER_ORDER) <= INITIAL_WALLET_BALANCE
            })
            .collect()
    }

    pub fn calculate_avg_yes_price(&self) -> Option<f64> {
        if self.filled_yes_orders.is_empty() {
            return None;
        }
        let avg = self.filled_yes_orders.iter().map(|o| o.price * o.contracts).sum::<f64>() / self.filled_yes_orders.iter().map(|o| o.contracts).sum::<f64>();
        Some(round_to_nearest_cent(avg))
    }

    pub fn calculate_avg_no_price(&self) -> Option<f64> {
        if self.filled_no_orders.is_empty() {
            return None;
        }
        let avg = self.filled_no_orders.iter().map(|o| o.price * o.contracts).sum::<f64>() / self.filled_no_orders.iter().map(|o| o.contracts).sum::<f64>();
        Some(round_to_nearest_cent(avg))
    }

    pub fn handle_ladders(&mut self, yes_ask: f64, no_ask: f64) {
        if self.yes_ladder_anchor_price.is_none() {
            if yes_ask < 0.60 && yes_ask > 0.0{
                self.yes_ladder_anchor_price = Some(yes_ask);
                self.yes_ladder = self.create_ladder(yes_ask, true);
                info!("Yes ladder: {:?}", self.yes_ladder);   
            }
        }
        if self.no_ladder_anchor_price.is_none() {
            if no_ask < 0.60 && no_ask > 0.0 {
                self.no_ladder_anchor_price = Some(no_ask);
                self.no_ladder = self.create_ladder(no_ask, false);
                info!("No ladder: {:?}", self.no_ladder);
            }
        }
    }

    pub fn handle_orders(&mut self, yes_ask: f64, no_ask: f64, timestamp: DateTime<Utc>) {
        let mut open_orders_dollars = 0.0;
        // if let Some(open_order) = &self.open_yes_rebalance {
        //     open_orders_dollars += open_order.price * open_order.contracts;
        //     if yes_ask < open_order.price {
        //         self.filled_yes_orders.push(FilledOrder {
        //             price: open_order.price,
        //             contracts: open_order.contracts,
        //             trade_type: TradeType::REBALANCE,
        //             trade_side: TradeSide::YES,
        //             timestamp,
        //         });
        //         self.open_yes_rebalance = None;
        //     }
        // }

        // if let Some(open_order) = &self.open_no_rebalance {
        //     open_orders_dollars += open_order.price * open_order.contracts;
        //     if no_ask < open_order.price {
        //         self.filled_no_orders.push(FilledOrder {
        //             price: open_order.price,
        //             contracts: open_order.contracts,
        //             trade_type: TradeType::REBALANCE,
        //             trade_side: TradeSide::NO,
        //             timestamp,
        //         });
        //         self.open_no_rebalance = None;
        //     }
        // }

        if self.balance < DOLLAR_PER_ORDER || self.balance < open_orders_dollars {
            return;
        }

        let mut filled_yes_prices = Vec::new();
        for yes_price in &self.yes_ladder {
            if yes_ask < *yes_price {
                let contracts = DOLLAR_PER_ORDER / *yes_price;
                self.balance -= DOLLAR_PER_ORDER;
                self.filled_yes_orders.push(FilledOrder {
                    price: *yes_price,
                    contracts,
                    trade_type: TradeType::LADDER,
                    trade_side: TradeSide::YES,
                    timestamp,
                });
                filled_yes_prices.push(*yes_price);
            }
        }
        self.yes_ladder.retain(|p| !filled_yes_prices.contains(p));

        let mut filled_no_prices = Vec::new();
        for no_price in &self.no_ladder {
            if no_ask < *no_price {
                let contracts = DOLLAR_PER_ORDER / *no_price;
                self.balance -= DOLLAR_PER_ORDER;
                self.filled_no_orders.push(FilledOrder {
                    price: *no_price,
                    contracts,
                    trade_type: TradeType::LADDER,
                    trade_side: TradeSide::NO,
                    timestamp,
                });
                filled_no_prices.push(*no_price);
            }
        }
        self.no_ladder.retain(|p| !filled_no_prices.contains(p));
    }

    // pub fn rebalance(&mut self) {
    //     let total_filled_yes_contracts = self
    //         .filled_yes_orders
    //         .iter()
    //         .map(|o| o.contracts)
    //         .sum::<f64>();
    //     let total_filled_no_contracts = self
    //         .filled_no_orders
    //         .iter()
    //         .map(|o| o.contracts)
    //         .sum::<f64>();

    //     let diff = total_filled_yes_contracts - total_filled_no_contracts;

    //     if diff == 0.0 {
    //         return;
    //     }

    //     if diff > 0.0 {
    //         self.open_yes_rebalance = None;

    //         let mut count = 0.0;
    //         let mut total_cost = 0.0;

    //         for order in self.filled_yes_orders.iter().rev() {
    //             let remaining = diff - count;
    //             if remaining <= 0.0 { break; }
                
    //             let take = order.contracts.min(remaining);
    //             total_cost += take * order.price;
    //             count += take;
    //         }

    //         self.open_no_rebalance = Some(OpenOrder {
    //             price: round_to_nearest_cent(REBALANCE_SUM - total_cost / diff),
    //             contracts: diff,
    //         });
    //     } else {
    //         self.open_no_rebalance = None;

    //         let mut count = 0.0;
    //         let mut total_cost = 0.0;

    //         for order in self.filled_no_orders.iter().rev() {
    //             let remaining = diff.abs() - count;
    //             if remaining <= 0.0 { break; }
                
    //             let take = order.contracts.min(remaining);
    //             total_cost += take * order.price;
    //             count += take;
    //         }

    //         self.open_yes_rebalance = Some(OpenOrder {
    //             price: round_to_nearest_cent(REBALANCE_SUM - total_cost / diff.abs()),
    //             contracts: diff.abs(),
    //         });
    //     }
    // }

    pub fn set_last_asks(&mut self, yes_ask: f64, no_ask: f64) {
        if yes_ask > 0.0 {
            self.last_yes_ask = Some(yes_ask);
        }
        if no_ask > 0.0 {
            self.last_no_ask = Some(no_ask);
        }
    }

    pub fn process_tick(&mut self, tick: &MarketDataRow) {
        self.handle_ladders(tick.yes_ask, tick.no_ask);
        self.handle_orders(tick.yes_ask, tick.no_ask, tick.timestamp);
        // self.rebalance();
        self.set_last_asks(tick.yes_ask, tick.no_ask);
    }

    pub fn reset(&mut self) {
        self.balance = INITIAL_WALLET_BALANCE;
        self.yes_ladder_anchor_price = None;
        self.no_ladder_anchor_price = None;
        self.yes_ladder = Vec::new();
        self.no_ladder = Vec::new();
        self.filled_yes_orders = Vec::new();
        self.filled_no_orders = Vec::new();
        self.last_no_ask = None;
        self.last_yes_ask = None;
    }

    pub fn filled_yes_contracts(&self) -> f64 {
        self.filled_yes_orders.iter().map(|o| o.contracts).sum()
    }

    pub fn filled_no_contracts(&self) -> f64 {
        self.filled_no_orders.iter().map(|o| o.contracts).sum()
    }

    // pub fn log_results(&self) {
    //     info!("Asset: {:?}", self.asset);
    //     info!("Balance remaining: {}", self.balance);

    //     let mut all_orders: Vec<&FilledOrder> = self.filled_yes_orders.iter()
    //         .chain(self.filled_no_orders.iter())
    //         .collect();

    //     all_orders.sort_by_key(|o| o.timestamp);

    //     for order in all_orders {
    //         info!("{:?}", order);
    //     }

    //     info!("Filled yes orders total contracts: {}", self.filled_yes_orders.iter().map(|o| o.contracts).sum::<f64>());
    //     info!("Filled no orders total contracts: {}", self.filled_no_orders.iter().map(|o| o.contracts).sum::<f64>());

    //     let avg_yes = match self.calculate_avg_yes_price() {
    //         Some(avg) => avg,
    //         None => 0.0,
    //     };
    //     let avg_no = match self.calculate_avg_no_price() {
    //         Some(avg) => avg,
    //         None => 0.0,
    //     };

    //     let total_cost = avg_yes + avg_no;

    //     info!("Avg price yes: {}", avg_yes);
    //     info!("Avg price no: {}", avg_no);

    //     info!("Total cost: {}", total_cost);
    // }

    pub fn append_result_to_csv(
        &self,
        path: &str,
        ticker: &str,
        total_rows_processed: usize
    ) -> std::io::Result<()> {
        let total_yes = self.filled_yes_contracts();
        let total_no = self.filled_no_contracts();
        let avg_yes = self.calculate_avg_yes_price().unwrap_or(0.0);
        let avg_no = self.calculate_avg_no_price().unwrap_or(0.0);
        let total_cost = round_to_nearest_cent(avg_yes + avg_no);

        let header = "ticker,total_rows_processed,balance_remaining,filled_yes_contracts,filled_no_contracts,avg_price_yes,avg_price_no,total_cost,last_yes_ask,last_no_ask\n";
        let row = format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            escape_csv_field(ticker),
            total_rows_processed,
            round_to_nearest_cent(self.balance),
            round_to_nearest_cent(total_yes),
            round_to_nearest_cent(total_no),
            avg_yes,
            avg_no,
            total_cost,
            self.last_yes_ask.unwrap_or(0.0),
            self.last_no_ask.unwrap_or(0.0),
        );

        let exists = std::path::Path::new(path).exists();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        if !exists {
            f.write_all(header.as_bytes())?;
        }
        f.write_all(row.as_bytes())?;
        Ok(())
    }

    pub async fn run(&mut self) {
        info!("Starting backtest...");

        dotenv::dotenv().ok();
        
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let db: Db = Db::new(&db_url).await.unwrap();

        let tickers: Vec<String> = db.fetch_all_tickers().await.unwrap();
        info!("Found {} tickers", tickers.len());

        let csv_path = "backtest_results.csv";

        for ticker in tickers.iter() {
            let market_data = db.fetch_ticker_market_data(&ticker).await.unwrap();
            info!("Found {} market data for ticker: {}", market_data.len(), ticker);

            self.asset = Some(market_data[0].asset.clone());
            self.reset();

            let total_rows = market_data.len();
            for tick in market_data.iter() {
                self.process_tick(tick);
            }

            self.append_result_to_csv(&csv_path, &ticker, total_rows)
                .expect("append backtest result to CSV");
        }

        // let csv_name = "2.csv";
        // let market_data = load_market_data_from_csv(csv_name).expect(&format!("failed to load {}", csv_name));
        // info!("Loaded {} rows from {}.csv", market_data.len(), csv_name);

        // if market_data.is_empty() {
        //     info!("No market data to process");
        //     return;
        // }

        // self.asset = market_data.first().map(|r| r.ticker.clone());
        // self.reset();

        // for tick in &market_data {
        //     self.process_tick(tick);
        // }

        // self.log_results();
    }
}

impl Default for BacktestEngine {
    fn default() -> Self {
        Self::new()
    }
}

// fn load_market_data_from_csv(path: &str) -> Result<Vec<MarketDataRow>, Box<dyn std::error::Error>> {
//     let file = File::open(path)?;
//     let reader = BufReader::new(file);
//     let mut rows = Vec::new();

//     for (i, line) in reader.lines().enumerate() {
//         let line = line?;
//         if i == 0 {
//             continue;
//         }
//         let parts: Vec<&str> = line.split(',').collect();
//         if parts.len() < 6 {
//             continue;
//         }
//         let timestamp = NaiveDateTime::parse_from_str(parts[0].trim(), "%Y-%m-%d %H:%M:%S")
//             .map(|dt| Utc.from_utc_datetime(&dt))
//             .map_err(|e| format!("parse timestamp {:?}: {}", parts[0], e))?;
//         let ticker = parts[1].trim().to_string();
//         let yes_ask: f64 = parts[2]
//             .trim()
//             .parse()
//             .map_err(|_| format!("parse yes_ask: {}", parts[2]))?;
//         let yes_bid: f64 = parts[3]
//             .trim()
//             .parse()
//             .map_err(|_| format!("parse yes_bid: {}", parts[3]))?;
//         let no_ask: f64 = parts[4]
//             .trim()
//             .parse()
//             .map_err(|_| format!("parse no_ask: {}", parts[4]))?;
//         let no_bid: f64 = parts[5]
//             .trim()
//             .parse()
//             .map_err(|_| format!("parse no_bid: {}", parts[5]))?;

//         rows.push(MarketDataRow {
//             timestamp,
//             ticker,
//             asset: String::new(),
//             yes_ask,
//             yes_bid,
//             no_ask,
//             no_bid,
//         });
//     }
//     Ok(rows)
// }
