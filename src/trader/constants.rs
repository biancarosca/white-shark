pub const TRADING_CHANNEL_BUFFER: usize = 256;

pub const MAX_BID_SUM: f64 = 0.97;
pub const REQUOTE_THRESHOLD: f64 = 0.02;
pub const INITIAL_CONTRACTS: u64 = 4;
pub const CONTRACTS_PER_ORDER: u64 = 2;
pub const MAX_IMBALANCE: i64 = 16;
pub const THROTTLE_MS: u64 = 1000;
pub const CANCEL_BEFORE_CLOSE_SECS: i64 = 5;
pub const NO_NEW_ANCHOR_BEFORE_CLOSE_SECS: i64 = 240;

// Used by models.rs for orderbook quantity checks
pub const FILL_OR_KILL_ORDER_PRICE: f64 = 0.99;

pub const MIN_REBALANCE_PRICE: f64 = 0.10;
pub const MIN_ANCHOR_PRICE: f64 = 0.90;
pub const ANCHOR_STEP_DOWN: f64 = 0.05;
pub const MAX_CANCEL_CHUNK_SIZE: usize = 20;