use std::env;

use tracing::info;

use white_shark::db::main::Db;
use white_shark::logging::init;

#[tokio::main]
async fn main() {
    init();

    let tickers: Vec<String> = env::args().skip(1).collect();
    if tickers.is_empty() {
        eprintln!("Usage: export_csv <TICKER1> [TICKER2] [TICKER3] ...");
        std::process::exit(1);
    }

    dotenv::dotenv().ok();
    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set in environment or .env");

    let db = Db::new(&database_url)
        .await
        .expect("Failed to connect to database");

    for ticker in &tickers {
        let csv_path = format!("{}.csv", ticker);
        let count = db
            .export_ticker_to_csv(ticker, &csv_path)
            .await
            .expect("Failed to export ticker data");

        info!("Exported {} rows for {} to {}", count, ticker, csv_path);
    }

    info!("Done — exported {} ticker(s)", tickers.len());
}
