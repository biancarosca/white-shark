use std::{fs::File, io::{BufRead, BufReader, Write}};

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Timelike, Utc};
use tracing::info;

use crate::db::main::{Db, MarketDataRow};

#[derive(Debug, Clone, Copy)]
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
    open_yes_order: Option<OpenOrder>,
    open_no_order: Option<OpenOrder>,
    filled_yes_orders: Vec<FilledOrder>,
    filled_no_orders: Vec<FilledOrder>,
    asset: Option<String>,
    last_yes_ask: Option<f64>,
    last_no_ask: Option<f64>,
    market_end: Option<DateTime<Utc>>,
}

const INITIAL_WALLET_BALANCE: f64 = 10000.0;
const CONTRACTS_PER_ORDER: f64 = 2.0;
const MAX_BID_SUM: f64 = 0.98;
const REQUOTE_THRESHOLD: f64 = 0.02;
const MAX_IMBALANCE: f64 = 2.0;
const MARKET_END_BUFFER_SECS: i64 = 240; // last 3 minutes of market data

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
            open_yes_order: None,
            open_no_order: None,
            filled_yes_orders: Vec::new(),
            filled_no_orders: Vec::new(),
            asset: None,
            last_yes_ask: None,
            last_no_ask: None,
            market_end: None,
        }
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

    fn check_fills(&mut self, yes_bid: f64, no_bid: f64, timestamp: DateTime<Utc>) {
        if let Some(order) = self.open_yes_order.take() {
            if yes_bid < order.price && yes_bid >= 0.0 {
                self.balance -= order.price * order.contracts;
                self.filled_yes_orders.push(FilledOrder {
                    price: order.price,
                    contracts: order.contracts,
                    trade_type: TradeType::LADDER,
                    trade_side: TradeSide::YES,
                    timestamp,
                });
            } else {
                self.open_yes_order = Some(order);
            }
        }

        if let Some(order) = self.open_no_order.take() {
            if no_bid < order.price && no_bid >= 0.0 {
                self.balance -= order.price * order.contracts;
                self.filled_no_orders.push(FilledOrder {
                    price: order.price,
                    contracts: order.contracts,
                    trade_type: TradeType::LADDER,
                    trade_side: TradeSide::NO,
                    timestamp,
                });
            } else {
                self.open_no_order = Some(order);
            }
        }
    }

    fn reserved_balance(&self) -> f64 {
        let yes_reserved = self.open_yes_order.as_ref()
            .map(|o| o.price * o.contracts).unwrap_or(0.0);
        let no_reserved = self.open_no_order.as_ref()
            .map(|o| o.price * o.contracts).unwrap_or(0.0);
        yes_reserved + no_reserved
    }

    fn place_orders(&mut self, yes_bid: f64, no_bid: f64) {
        if yes_bid <= 0.0 || no_bid <= 0.0 {
            return;
        }

        let bid_sum = yes_bid + no_bid;
        if bid_sum > MAX_BID_SUM {
            self.open_yes_order = None;
            self.open_no_order = None;
            return;
        }

        let imbalance = self.get_contract_diff();
        let last_yes = self.filled_yes_orders.last().map(|o| o.price);
        let last_no = self.filled_no_orders.last().map(|o| o.price);

        let yes_at_cap = imbalance >= MAX_IMBALANCE;
        let no_at_cap = imbalance <= -MAX_IMBALANCE;

        let (quote_yes, quote_no) = if yes_bid > no_bid {
            (!yes_at_cap, imbalance > 0.0)
        } else if no_bid > yes_bid {
            (imbalance < 0.0, !no_at_cap)
        } else {
            (!yes_at_cap, !no_at_cap)
        };

        if quote_yes {
            let pair_ok = match last_no {
                Some(np) if imbalance < 0.0 => yes_bid + np <= MAX_BID_SUM,
                _ => bid_sum <= MAX_BID_SUM,
            };
            let needs_requote = match &self.open_yes_order {
                None => true,
                Some(o) => (o.price - yes_bid).abs() >= REQUOTE_THRESHOLD,
            };
            if pair_ok && needs_requote && self.balance >= yes_bid * CONTRACTS_PER_ORDER {
                self.open_yes_order = Some(OpenOrder { price: yes_bid, contracts: CONTRACTS_PER_ORDER });
            } else if !pair_ok {
                self.open_yes_order = None;
            }
        } else {
            self.open_yes_order = None;
        }

        if quote_no {
            let pair_ok = match last_yes {
                Some(yp) if imbalance > 0.0 => yp + no_bid <= MAX_BID_SUM,
                _ => bid_sum <= MAX_BID_SUM,
            };
            let needs_requote = match &self.open_no_order {
                None => true,
                Some(o) => (o.price - no_bid).abs() >= REQUOTE_THRESHOLD,
            };
            if pair_ok && needs_requote && self.balance >= no_bid * CONTRACTS_PER_ORDER {
                self.open_no_order = Some(OpenOrder { price: no_bid, contracts: CONTRACTS_PER_ORDER });
            } else if !pair_ok {
                self.open_no_order = None;
            }
        } else {
            self.open_no_order = None;
        }
    }

    pub fn get_contract_diff(&self) -> f64 {
        let total_filled_yes_contracts = self
            .filled_yes_orders
            .iter()
            .map(|o| o.contracts)
            .sum::<f64>();
        let total_filled_no_contracts = self
            .filled_no_orders.iter().map(|o| o.contracts).sum::<f64>();
        total_filled_yes_contracts - total_filled_no_contracts
    }

    pub fn set_last_asks(&mut self, yes_ask: f64, no_ask: f64) {
        if yes_ask > 0.0 {
            self.last_yes_ask = Some(yes_ask);
        }
        if no_ask > 0.0 {
            self.last_no_ask = Some(no_ask);
        }
    }

    pub fn process_tick(&mut self, tick: &MarketDataRow) {
        self.check_fills(tick.yes_bid, tick.no_bid, tick.timestamp);

        let near_market_end = self.market_end
            .map(|end| tick.timestamp >= end - Duration::seconds(MARKET_END_BUFFER_SECS))
            .unwrap_or(false);

        if !near_market_end {
            self.place_orders(tick.yes_bid, tick.no_bid);
        }

        self.set_last_asks(tick.yes_ask, tick.no_ask);
    }

    pub fn reset(&mut self) {
        self.balance = INITIAL_WALLET_BALANCE;
        self.open_yes_order = None;
        self.open_no_order = None;
        self.filled_yes_orders = Vec::new();
        self.filled_no_orders = Vec::new();
        self.last_no_ask = None;
        self.last_yes_ask = None;
        self.market_end = None;
    }

    pub fn filled_yes_contracts(&self) -> f64 {
        self.filled_yes_orders.iter().map(|o| o.contracts).sum()
    }

    pub fn filled_no_contracts(&self) -> f64 {
        self.filled_no_orders.iter().map(|o| o.contracts).sum()
    }

    pub fn log_results(&self) {
        let total_yes = self.filled_yes_contracts();
        let total_no = self.filled_no_contracts();
        let avg_yes = self.calculate_avg_yes_price().unwrap_or(0.0);
        let avg_no = self.calculate_avg_no_price().unwrap_or(0.0);
        let avg_sum = round_to_nearest_cent(avg_yes + avg_no);
        let imbalance = self.get_contract_diff();

        info!("--- Results ---");
        info!("Balance remaining: ${:.2}", self.balance);
        info!("YES fills: {:.1} contracts @ avg ${:.2}", total_yes, avg_yes);
        for order in self.filled_yes_orders.iter() {
            info!("  - {:.1} contracts @ ${:.2} at {}", order.contracts, order.price, order.timestamp);
        }
        info!("NO  fills: {:.1} contracts @ avg ${:.2}", total_no, avg_no);
        for order in self.filled_no_orders.iter() {
            info!("  - {:.1} contracts @ ${:.2} at {}", order.contracts, order.price, order.timestamp);
        }
        info!("Avg sum: ${:.2} ({})", avg_sum, if avg_sum < 1.0 { "profitable" } else { "underwater" });
        info!("Imbalance: {:.1} contracts ({})", imbalance.abs(),
            if imbalance > 0.0 { "YES heavy" } else if imbalance < 0.0 { "NO heavy" } else { "balanced" });

        let matched = total_yes.min(total_no);
        let spread_pnl = matched * (1.0 - avg_sum);
        info!("Matched pairs: {:.1} → spread P&L: ${:.2}", matched, spread_pnl);

        let residual = (total_yes - total_no).abs();
        if residual > 0.0 {
            let residual_side = if imbalance > 0.0 { "YES" } else { "NO" };
            let residual_avg = if imbalance > 0.0 { avg_yes } else { avg_no };
            info!("Residual: {:.1} {} contracts @ avg ${:.2} (need resolution)", residual, residual_side, residual_avg);
        }

        info!("Total fills: {} YES + {} NO", self.filled_yes_orders.len(), self.filled_no_orders.len());
        info!("Last YES ask: {}", self.last_yes_ask.unwrap_or(0.0));
        info!("Last NO ask: {}", self.last_no_ask.unwrap_or(0.0));
        info!("Open YES order: {:?}", self.open_yes_order);
        info!("Open NO order: {:?}", self.open_no_order);
    }

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
        let avg_sum = round_to_nearest_cent(avg_yes + avg_no);
        let matched = total_yes.min(total_no);
        let spread_pnl = matched * (1.0 - avg_sum);
        let imbalance = self.get_contract_diff();

        let header = "ticker,total_rows,yes_contracts,no_contracts,avg_yes,avg_no,avg_sum,matched,spread_pnl,imbalance,last_yes_ask,last_no_ask\n";
        let row = format!(
            "{},{},{:.1},{:.1},{:.2},{:.2},{:.2},{:.1},{:.2},{:.1},{},{}\n",
            escape_csv_field(ticker),
            total_rows_processed,
            total_yes,
            total_no,
            avg_yes,
            avg_no,
            avg_sum,
            matched,
            spread_pnl,
            imbalance,
            self.last_yes_ask.map(|t| t.to_string()).unwrap_or_default(),
            self.last_no_ask.map(|t| t.to_string()).unwrap_or_default(),
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

        if tickers.is_empty() {
            info!("No tickers found");
            return;
        }

        let csv_path = "backtest_results.csv";

        for ticker in tickers.iter().take(100) {
            let market_data = db.fetch_ticker_market_data(&ticker).await.unwrap();
            info!("Found {} market data for ticker: {}", market_data.len(), ticker);

            if market_data.is_empty() {
                info!("Skipping ticker {} — no market data", ticker);
                continue;
            }

            self.asset = Some(market_data[0].asset.clone());
            self.reset();
            
            let first_timestamp = market_data[0].timestamp;
            let fifteen_min_from_first_timestamp = first_timestamp + Duration::minutes(15);
            self.market_end = Some(fifteen_min_from_first_timestamp);

            let total_rows = market_data.len();
            for tick in market_data.iter() {
                self.process_tick(tick);
            }

            self.append_result_to_csv(&csv_path, &ticker, total_rows)
                .expect("append backtest result to CSV");
        }


        // let csv_name = "3.csv";
        // let market_data = load_market_data_from_csv(csv_name).expect(&format!("failed to load {}", csv_name));
        // info!("Loaded {} rows from {}.csv", market_data.len(), csv_name);

        // if market_data.is_empty() {
        //     info!("No market data to process");
        //     return;
        // }

        // // self.asset = market_data.first().map(|r| r.ticker.clone());
        // // self.reset();

        // let first_timestamp = market_data.first().map(|r| r.timestamp).unwrap();
        // let boundary_minute = (first_timestamp.minute() / 15) * 15;
        // let market_end = first_timestamp
        //     .with_minute(boundary_minute).unwrap()
        //     .with_second(0).unwrap()
        //     .with_nanosecond(0).unwrap()
        //     + Duration::minutes(15);
        // self.market_end = Some(market_end);

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
