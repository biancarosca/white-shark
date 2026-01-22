use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::error::Result;
use crate::event_processor::process_events;
use crate::exchanges::binance::BinanceClient;
use crate::exchanges::kalshi::{KalshiClient, KalshiEvent};
use crate::exchanges::PriceUpdate;

pub async fn run(config: Config) -> Result<()> {
    info!("🦈 Started");
    info!("================================");

    info!("Kalshi symbols: {:?}", config.kalshi.tracked_symbols);
    info!("Binance symbols: {:?}", config.binance.tracked_symbols);

    //let (kalshi_tx, kalshi_rx) = mpsc::channel::<KalshiEvent>(100);
    let (binance_tx, binance_rx) = mpsc::channel::<PriceUpdate>(100);

    // let kalshi_config = config.kalshi.clone();
    // let kalshi_handle = tokio::spawn(async move {
    //     match KalshiClient::new(kalshi_config) {
    //         Ok(mut client) => {
    //             if let Err(e) = client.start(kalshi_tx).await {
    //                 error!("Kalshi error: {}", e);
    //             }
    //         }
    //         Err(e) => error!("Failed to create Kalshi client: {}", e),
    //     }
    // });

    let binance_config = config.binance.clone();
    let binance_handle = tokio::spawn(async move {
        let mut client = BinanceClient::new(binance_config.clone()).with_sbe();
        if let Err(e) = client.start(&binance_config.tracked_symbols, binance_tx).await {
            error!("Binance error: {}", e);
        }
    });

    //let event_handle = tokio::spawn(process_events(kalshi_rx, binance_rx));
    let event_handle = tokio::spawn(process_events(binance_rx));

    let _ = tokio::try_join!(binance_handle, event_handle);

    Ok(())
}

