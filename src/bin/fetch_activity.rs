use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;

use chrono::DateTime;
use serde::Deserialize;
use tracing::{info, warn};

use white_shark::logging::init;

const USER_ADDRESS: &str = "0x6031b6eed1c97e853c6e0f03ad3ce3529351f96d";
const PAGE_LIMIT: u64 = 1000;
const MAX_RECORDS: u64 = 30_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Activity {
    #[serde(default)]
    outcome: String,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    size: f64,
    timestamp: i64,
    slug: String,
    #[serde(rename = "type")]
    activity_type: String,
}

#[derive(Debug, Deserialize)]
struct PastResultsResponse {
    status: String,
    data: Option<PastResultsData>,
}

#[derive(Debug, Deserialize)]
struct PastResultsData {
    results: Vec<PastResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PastResult {
    start_time: String,
    outcome: String,
}

/// Parse a market slug into (symbol, api_variant, start_unix_ts).
fn parse_slug(slug: &str, trades: &[&Activity]) -> Option<(String, String, i64)> {
    let parts: Vec<&str> = slug.split('-').collect();

    // Structured format: {symbol}-updown-{interval}-{timestamp}
    if parts.len() == 4 && parts[1] == "updown" {
        let symbol = parts[0].to_uppercase();
        let variant = match parts[2] {
            "5m" => "fiveminute",
            "15m" => "fifteen",
            _ => return None,
        };
        let ts: i64 = parts[3].parse().ok()?;
        return Some((symbol, variant.to_string(), ts));
    }

    // Hourly format: bitcoin-up-or-down-* / ethereum-up-or-down-*
    if slug.starts_with("bitcoin-up-or-down") || slug.starts_with("ethereum-up-or-down") {
        let symbol = if slug.starts_with("bitcoin") { "BTC" } else { "ETH" };
        let min_ts = trades.iter().map(|t| t.timestamp).min()?;
        let start_ts = min_ts - (min_ts % 3600);
        return Some((symbol.to_string(), "hourly".to_string(), start_ts));
    }

    None
}

async fn fetch_market_outcomes(
    client: &reqwest::Client,
    markets: &HashMap<&str, Vec<&Activity>>,
) -> HashMap<i64, String> {
    // Collect (symbol, variant, start_ts) for each market we can parse
    let mut groups: HashMap<(String, String), Vec<i64>> = HashMap::new();
    for (slug, trades) in markets {
        if let Some((symbol, variant, start_ts)) = parse_slug(slug, trades) {
            groups.entry((symbol, variant)).or_default().push(start_ts);
        }
    }

    let mut outcomes: HashMap<i64, String> = HashMap::new();

    for ((symbol, variant), timestamps) in &groups {
        let max_ts = *timestamps.iter().max().unwrap();
        let buffer = match variant.as_str() {
            "fiveminute" => 300,
            "fifteen" => 900,
            "hourly" => 3600,
            _ => 3600,
        };
        let end_time = DateTime::from_timestamp(max_ts + buffer, 0).unwrap();
        let count = timestamps.len() + 10;

        let url = format!(
            "https://polymarket.com/api/past-results?symbol={}&variant={}&assetType=crypto&currentEventStartTime={}&count={}",
            symbol, variant, end_time.format("%Y-%m-%dT%H:%M:%SZ"), count
        );

        info!("Fetching past results for {} {} ...", symbol, variant);

        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to fetch past results for {} {}: {}", symbol, variant, e);
                continue;
            }
        };

        let body: PastResultsResponse = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to parse past results for {} {}: {}", symbol, variant, e);
                continue;
            }
        };

        if body.status != "success" {
            warn!("Past results API returned status '{}' for {} {}", body.status, symbol, variant);
            continue;
        }

        if let Some(data) = body.data {
            for r in data.results {
                if let Ok(dt) = DateTime::parse_from_rfc3339(&r.start_time) {
                    outcomes.insert(dt.timestamp(), r.outcome);
                }
            }
            info!("Got {} past results for {} {}", outcomes.len(), symbol, variant);
        }
    }

    outcomes
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init();

    let client = reqwest::Client::new();
    let mut all_activities: Vec<Activity> = Vec::new();
    let mut offset: u64 = 0;
    let mut exhausted = true;

    loop {
        if offset >= MAX_RECORDS {
            info!("Reached max records limit of {}", MAX_RECORDS);
            exhausted = false;
            break;
        }

        let url = format!(
            "https://data-api.polymarket.com/activity?user={}&limit={}&offset={}",
            USER_ADDRESS, PAGE_LIMIT, offset
        );

        info!("Fetching offset={} ...", offset);

        let body: serde_json::Value = client.get(&url).send().await?.json().await?;
        let resp: Vec<Activity> = match body {
            serde_json::Value::Array(_) => serde_json::from_value(body)?,
            _ => {
                info!("API returned non-array response, stopping pagination");
                break;
            }
        };
        let count = resp.len() as u64;
        info!("Got {} records", count);

        if count == 0 {
            break;
        }

        all_activities.extend(resp);
        offset += count;

        if count < PAGE_LIMIT {
            break;
        }
    }

    let last_page_size = if all_activities.is_empty() {
        0
    } else {
        PAGE_LIMIT.min(all_activities.len() as u64)
    };
    let incomplete_slugs: HashSet<String> = if exhausted {
        HashSet::new()
    } else {
        let boundary = (all_activities.len() as u64).saturating_sub(last_page_size) as usize;
        all_activities[boundary..]
            .iter()
            .map(|a| a.slug.clone())
            .collect()
    };

    if !incomplete_slugs.is_empty() {
        warn!(
            "Skipping {} market(s) that may be incomplete: {:?}",
            incomplete_slugs.len(),
            incomplete_slugs
        );
    }

    let trades: Vec<&Activity> = all_activities
        .iter()
        .filter(|a| a.activity_type == "TRADE" && !incomplete_slugs.contains(&a.slug))
        .collect();

    info!(
        "Total records fetched: {} ({} trades to export)",
        all_activities.len(),
        trades.len()
    );

    let mut markets: HashMap<&str, Vec<&Activity>> = HashMap::new();
    for trade in &trades {
        markets.entry(&trade.slug).or_default().push(trade);
    }

    let outcomes = fetch_market_outcomes(&client, &markets).await;

    fs::create_dir_all("data")?;

    for (ticker, activities) in &markets {
        let path = format!("data/{}.csv", ticker);
        let mut file = fs::File::create(&path)?;
        writeln!(file, "outcome,price,size,date")?;

        let mut total_up: f64 = 0.0;
        let mut total_down: f64 = 0.0;
        let mut sum_price_up: f64 = 0.0;
        let mut sum_price_down: f64 = 0.0;
        let mut count_up: u64 = 0;
        let mut count_down: u64 = 0;

        for a in activities {
            let date = DateTime::from_timestamp(a.timestamp, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| a.timestamp.to_string());

            match a.outcome.as_str() {
                "Up" => {
                    total_up += a.size;
                    sum_price_up += a.price;
                    count_up += 1;
                }
                "Down" => {
                    total_down += a.size;
                    sum_price_down += a.price;
                    count_down += 1;
                }
                _ => {}
            }

            writeln!(file, "{},{},{},{}", a.outcome, a.price, a.size, date)?;
        }

        let avg_up = if count_up > 0 {
            sum_price_up / count_up as f64
        } else {
            0.0
        };
        let avg_down = if count_down > 0 {
            sum_price_down / count_down as f64
        } else {
            0.0
        };

        writeln!(file, "total up,,,{}", total_up)?;
        writeln!(file, "total down,,,{}", total_down)?;
        writeln!(file, "avg up price,{}", avg_up)?;
        writeln!(file, "avg down price,{}", avg_down)?;

        if let Some((_, _, start_ts)) = parse_slug(ticker, activities) {
            if let Some(resolved) = outcomes.get(&start_ts) {
                writeln!(file, "market outcome,{}", resolved)?;
            } else {
                writeln!(file, "market outcome,unknown")?;
            }
        }

        info!(
            "Wrote {} rows to {} (up: {:.2}, down: {:.2})",
            activities.len(),
            path,
            total_up,
            total_down
        );
    }

    info!("Done — exported {} market(s)", markets.len());

    Ok(())
}
