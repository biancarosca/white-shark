pub const BATCH_SIZE: usize = 1000;
pub const FLUSH_INTERVAL_MS: u64 = 5000;
pub const CHANNEL_BUFFER_SIZE: usize = 50000;
pub const FETCH_AFTER_CLOSE_SECS: i64 = 3;

pub const INITIAL_BACKOFF_SECS: u64 = 1;
pub const MAX_BACKOFF_SECS: u64 = 60;

pub const WS_IDLE_RECONNECT_SECS: u64 = 10;

pub const MAX_MARKET_FETCH_ATTEMPTS: u64 = 20;
pub const MARKET_FETCH_INTERVAL_SECS: u64 = 10;
