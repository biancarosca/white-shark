use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::error::Result;
use crate::event_processor::process_events;
use crate::exchanges::binance::client::BinanceClient;
use crate::exchanges::kalshi::{KalshiClient, KalshiEvent};
use crate::exchanges::PriceUpdate;

pub async fn run(config: Config) -> Result<()> {
    info!("🦈 Started");
    info!("================================");

    info!("Kalshi symbols: {:?}", config.kalshi.tracked_symbols);
    info!("Binance symbols: {:?}", config.binance.tracked_symbols);

    let (kalshi_tx, kalshi_rx) = mpsc::channel::<KalshiEvent>(100);
    let (binance_tx, binance_rx) = mpsc::channel::<PriceUpdate>(100);
    let (imbalance_tx, imbalance_rx) = mpsc::channel::<crate::event_processor::ImbalanceAlert>(100);

    use std::sync::Arc;
    use crate::state::KalshiState;
    
    // Create shared state that both client and event processor will use
    let shared_state = Arc::new(KalshiState::new());
    
    let kalshi_config = config.kalshi.clone();
    let kalshi_state_for_client = shared_state.clone();
    let mut kalshi_client = KalshiClient::new(kalshi_config, kalshi_state_for_client)?;
    
    let kalshi_handle = tokio::spawn(async move {
        if let Err(e) = kalshi_client.start(kalshi_tx).await {
            error!("Kalshi error: {}", e);
        }
    });

    let binance_config = config.binance.clone();
    let imbalance_tx_for_binance = imbalance_tx.clone();
    let binance_handle = tokio::spawn(async move {
        let mut client = BinanceClient::new(binance_config.clone());
        client.set_imbalance_tx(imbalance_tx_for_binance);
        if let Err(e) = client.start(&binance_config.tracked_symbols, binance_tx).await {
            error!("Binance error: {}", e);
        }
    });

    let event_handle = tokio::spawn(process_events(binance_rx, kalshi_rx, imbalance_rx, shared_state));

    let _ = tokio::try_join!(binance_handle, kalshi_handle, event_handle);

    Ok(())
}

