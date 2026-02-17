use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::db::main::Db;
use crate::error::Result;
use crate::exchanges::binance::client::BinanceClient;
use crate::exchanges::kalshi::KalshiClient;

pub async fn run(config: Config) -> Result<()> {
    info!("🦈 Started");
    info!("================================");

    let db = Arc::new(Db::new(&config.database.url).await?);

    db.export_ticker_to_csv("KXBTC15M-26FEB072045-45", "btc_1.csv").await?;
    db.export_ticker_to_csv("KXETH15M-26FEB072045-45", "eth_1.csv").await?;
    db.export_ticker_to_csv("KXSOL15M-26FEB072045-45", "sol_1.csv").await?;

    db.export_ticker_to_csv("KXBTC15M-26FEB122330-30", "btc_1.csv").await?;
    db.export_ticker_to_csv("KXETH15M-26FEB122330-30", "eth_2.csv").await?;
    db.export_ticker_to_csv("KXSOL15M-26FEB122330-30", "sol_3.csv").await?;

    // info!("Kalshi symbols: {:?}", config.kalshi.tracked_symbols);

    // let (price_tx, _price_rx) = mpsc::channel(128);

    // let kalshi_config = config.kalshi.clone();
    // let mut kalshi_client = KalshiClient::new(kalshi_config, db)?;

    // // if let Err(e) = kalshi_client.start().await {
    // //     error!("Kalshi client error: {}", e);
    // // }

    // let kalshi_handle = tokio::spawn(async move {
    //     if let Err(e) = kalshi_client.start().await {
    //         error!("Kalshi client error: {}", e);
    //     }
    // });

    // let binance_symbols = config.binance.tracked_symbols.clone();
    // let mut binance = BinanceClient::new(config.binance.clone());

    // let binance_handle = tokio::spawn(async move {
    //     if let Err(e) = binance.start(&binance_symbols, price_tx).await {
    //         error!("Binance client error: {}", e);
    //     }
    // });

    // //Wait for both — if one crashes, the other keeps running
    // let _ = tokio::join!(kalshi_handle, binance_handle);

    Ok(())
}

