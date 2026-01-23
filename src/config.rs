use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub kalshi: KalshiConfig,
    pub binance: BinanceConfig,
}

#[derive(Debug, Clone)]
pub struct KalshiConfig {
    pub api_key_id: String,
    pub private_key_path: String,
    pub tracked_symbols: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BinanceConfig {
    pub api_key: Option<String>,
    pub tracked_symbols: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let kalshi_api_key = std::env::var("KALSHI_API_KEY_ID")
            .map_err(|_| Error::Config("KALSHI_API_KEY_ID not set".into()))?;

        let kalshi_private_key_path = std::env::var("KALSHI_PRIVATE_KEY_PATH")
            .map_err(|_| Error::Config("KALSHI_PRIVATE_KEY_PATH not set".into()))?;

        let kalshi_symbols = std::env::var("KALSHI_TRACKED_SYMBOLS")
            .map_err(|_| Error::Config("KALSHI_TRACKED_SYMBOLS not set".into()))?
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .collect();

        let binance_api_key = std::env::var("BINANCE_API_KEY").ok();

        let binance_symbols = std::env::var("BINANCE_TRACKED_SYMBOLS")
            .map_err(|_| Error::Config("BINANCE_TRACKED_SYMBOLS not set".into()))?
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .collect();

        Ok(Config {
            kalshi: KalshiConfig {
                api_key_id: kalshi_api_key,
                private_key_path: kalshi_private_key_path,
                tracked_symbols: kalshi_symbols,
            },
            binance: BinanceConfig {
                api_key: binance_api_key,
                tracked_symbols: binance_symbols
            },
        })
    }
}

impl Default for KalshiConfig {
    fn default() -> Self {
        Self {
            api_key_id: String::new(),
            private_key_path: "private_key.pem".to_string(),
            tracked_symbols: vec!["ETH15M".to_string(), "BTC15M".to_string()],
        }
    }
}

impl Default for BinanceConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            tracked_symbols: vec!["ETHUSDT".to_string(), "BTCUSDT".to_string()],
        }
    }
}

