use std::sync::Arc;

use tracing::{error, info};

use crate::config::Config;
use crate::db::main::Db;
use crate::error::Result;
use crate::exchanges::kalshi::KalshiClient;

pub async fn run(config: Config) -> Result<()> {
    info!("🦈 Started");
    info!("================================");

    let db = Arc::new(Db::new(&config.database.url).await?);

    info!("Kalshi symbols: {:?}", config.kalshi.tracked_symbols);

    let kalshi_config = config.kalshi.clone();
    let mut kalshi_client = KalshiClient::new(kalshi_config, db)?;

    if let Err(e) = kalshi_client.start().await {
        error!("Kalshi client error: {}", e);
    }

    Ok(())
}

