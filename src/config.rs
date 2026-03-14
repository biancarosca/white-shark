use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub kalshi: KalshiConfig,
    // pub binance: BinanceConfig,
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone)]
pub struct KalshiConfig {
    pub api_key_id: String,
    /// PEM content directly (preferred for deployment)
    pub private_key: Option<String>,
    /// Path to PEM file (fallback for local development)
    pub private_key_path: Option<String>,
    pub tracked_symbols: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BinanceConfig {
    pub api_key: Option<String>,
    pub tracked_symbols: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let kalshi_api_key = std::env::var("KALSHI_API_KEY_ID")
            .map_err(|_| Error::Config("KALSHI_API_KEY_ID not set".into()))?;

        // Try KALSHI_PRIVATE_KEY (content) first, then fall back to KALSHI_PRIVATE_KEY_PATH (file)
        let kalshi_private_key = std::env::var("KALSHI_PRIVATE_KEY").ok();
        let kalshi_private_key_path = std::env::var("KALSHI_PRIVATE_KEY_PATH").ok();

        if kalshi_private_key.is_none() && kalshi_private_key_path.is_none() {
            return Err(Error::Config(
                "Either KALSHI_PRIVATE_KEY or KALSHI_PRIVATE_KEY_PATH must be set".into()
            ));
        }

        let kalshi_symbols = std::env::var("KALSHI_TRACKED_SYMBOLS")
            .map_err(|_| Error::Config("KALSHI_TRACKED_SYMBOLS not set".into()))?
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .collect();

        // let binance_api_key = std::env::var("BINANCE_API_KEY").ok();

        // let binance_symbols = std::env::var("BINANCE_TRACKED_SYMBOLS")
        //     .map_err(|_| Error::Config("BINANCE_TRACKED_SYMBOLS not set".into()))?
        //     .split(',')
        //     .map(|s| s.trim().to_uppercase())
        //     .collect();

        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| Error::Config("DATABASE_URL not set".into()))?;

        Ok(Config {
            kalshi: KalshiConfig {
                api_key_id: kalshi_api_key,
                private_key: kalshi_private_key,
                private_key_path: kalshi_private_key_path,
                tracked_symbols: kalshi_symbols,
            },
            // binance: BinanceConfig {
            //     api_key: binance_api_key,
            //     tracked_symbols: binance_symbols,
            // },
            database: DatabaseConfig {
                url: database_url,
            },
        })
    }
}

impl Default for KalshiConfig {
    fn default() -> Self {
        Self {
            api_key_id: String::new(),
            private_key: None,
            private_key_path: Some("private_key.pem".to_string()),
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

