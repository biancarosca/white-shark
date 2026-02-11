use tracing::info;

use white_shark::backtest::engine::BacktestEngine;
use white_shark::logging::init;

#[tokio::main]
async fn main() {
    init();

    let mut engine = BacktestEngine::new();
    engine.run().await;
    info!("Backtest finished.");
}
