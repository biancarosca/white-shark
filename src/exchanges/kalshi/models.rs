use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;

#[derive(Debug, Serialize)]
pub struct SubscribeMessage {
    pub id: u64,
    pub cmd: String,
    pub params: SubscribeParams,
}

#[derive(Debug, Serialize)]
pub struct SubscribeParams {
    pub channels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_tickers: Option<Vec<String>>,
}

impl SubscribeMessage {
    pub fn new(id: u64, channels: Vec<String>, market_tickers: Option<Vec<String>>) -> Self {
        Self {
            id,
            cmd: "subscribe".to_string(),
            params: SubscribeParams {
                channels,
                market_tickers,
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UnsubscribeParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sids: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_tickers: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct UnsubscribeMessage {
    pub id: u64,
    pub cmd: String,
    pub params: UnsubscribeParams,
}

impl UnsubscribeMessage {
    pub fn new(id: u64, sids: Vec<u64>) -> Self {
        Self {
            id,
            cmd: "unsubscribe".to_string(),
            params: UnsubscribeParams {
                sids: Some(sids),
                channels: None,
                market_tickers: None,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct KalshiWsMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub sid: Option<u64>,
    pub msg: Option<serde_json::Value>,
    pub data: Option<serde_json::Value>,
    pub status: Option<String>,
    pub error: Option<String>,
}

impl KalshiWsMessage {
    pub fn payload(&self) -> Option<&serde_json::Value> {
        self.msg.as_ref().or(self.data.as_ref())
    }

    pub fn is_subscribed(&self) -> bool {
        self.msg_type.as_deref() == Some("subscribed")
    }

}


#[derive(Debug, Clone)]
pub enum KalshiEvent {
    MarketStatusChanged {
        ticker: String,
        old_status: Option<KalshiMarketStatus>,
        new_status: KalshiMarketStatus,
    },
    TickerUpdate(KalshiTicker),
    OrderbookUpdate(KalshiOrderbook),
    OrderbookDelta(KalshiOrderbookDelta),
    Trade(KalshiTrade),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiTicker {
    pub market_ticker: String,
    #[serde(default)]
    pub price: Option<i64>, // Last traded price in cents (1-99)
    #[serde(default)]
    pub yes_bid: Option<i64>, // Best bid price for yes side in cents
    #[serde(default)]
    pub yes_ask: Option<i64>, // Best ask price for yes side in cents
    #[serde(default)]
    pub price_dollars: Option<String>, // Last traded price in dollars
    #[serde(default)]
    pub yes_bid_dollars: Option<String>, // Best bid price for yes side in dollars
    #[serde(default)]
    pub no_bid_dollars: Option<String>, // Best bid price for no side in dollars
    #[serde(default)]
    pub volume: Option<i64>, // Number of individual contracts traded
    #[serde(default)]
    pub volume_fp: Option<String>, // Fixed-point total contracts traded (2 decimals)
    #[serde(default)]
    pub open_interest: Option<i64>, // Number of active contracts
    #[serde(default)]
    pub open_interest_fp: Option<String>, // Fixed-point open interest (2 decimals)
    #[serde(default)]
    pub dollar_volume: Option<i64>, // Number of dollars traded
    #[serde(default)]
    pub dollar_open_interest: Option<i64>, // Number of dollars positioned
    #[serde(default)]
    pub ts: Option<i64>, // Unix timestamp in seconds
}

impl KalshiTicker {
    /// Get YES ask price as f64 (from cents converted to decimal)
    pub fn yes_ask_f64(&self) -> Option<f64> {
        // Convert cents to decimal (cents / 100)
        self.yes_ask.map(|cents| cents as f64 / 100.0)
    }

    /// Get YES bid price as f64 (from dollars string or cents converted to decimal)
    pub fn yes_bid_f64(&self) -> Option<f64> {
        // Try dollars string first
        self.yes_bid_dollars
            .as_ref()
            .and_then(|d| d.parse::<f64>().ok())
            .or_else(|| {
                // Fall back to cents converted to decimal (cents / 100)
                self.yes_bid.map(|cents| cents as f64 / 100.0)
            })
    }

    /// Get NO bid price as f64 (from dollars string)
    pub fn no_bid_f64(&self) -> Option<f64> {
        self.no_bid_dollars
            .as_ref()
            .and_then(|s| s.parse::<f64>().ok())
    }

    /// Get NO ask price as f64 (inferred from YES bid: 1 - yes_bid)
    pub fn no_ask_f64(&self) -> Option<f64> {
        self.yes_bid_f64().map(|yes_bid| 1.0 - yes_bid)
    }

    /// Get last price as f64 (from dollars string or cents converted to decimal)
    pub fn price_f64(&self) -> Option<f64> {
        self.price_dollars
            .as_ref()
            .and_then(|d| d.parse::<f64>().ok())
            .or_else(|| {
                self.price.map(|cents| cents as f64 / 100.0)
            })
    }

    /// Get timestamp as DateTime<Utc>
    pub fn timestamp(&self) -> Option<DateTime<Utc>> {
        self.ts.and_then(|ts| DateTime::from_timestamp(ts, 0))
    }

    pub fn implied_no_ask(&self) -> Option<f64> {
        self.yes_bid_f64().map(|yb| 1.0 - yb)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiMarket {
    pub ticker: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: KalshiMarketStatus,
    pub open_time: Option<String>,
    pub close_time: Option<String>,
    pub expiration_time: Option<String>,
    pub settlement_timer_seconds: Option<i64>,
    pub yes_bid: Option<f64>,
    pub yes_ask: Option<f64>,
    pub no_bid: Option<f64>,
    pub no_ask: Option<f64>,
    pub last_price: Option<f64>,
    pub volume: Option<i64>,
    pub volume_24h: Option<i64>,
    pub open_interest: Option<i64>,
    pub category: Option<String>,
    pub series_ticker: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}


#[derive(Debug, Deserialize)]
pub struct MarketsResponse {
    pub markets: Vec<KalshiMarket>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiOrderbook {
    pub market_ticker: String,
    #[serde(default)]
    pub yes_bids: Vec<OrderbookLevel>,
    #[serde(default)]
    pub yes_asks: Vec<OrderbookLevel>,
    #[serde(default)]
    pub no_bids: Vec<OrderbookLevel>,
    #[serde(default)]
    pub no_asks: Vec<OrderbookLevel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiOrderbookSnapshot {
    pub market_ticker: String,
    #[serde(default)]
    pub yes_dollars: Vec<(String, i64)>,
    #[serde(default)]
    pub no_dollars: Vec<(String, i64)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiOrderbookDelta {
    pub market_ticker: String,
    pub price_dollars: String,
    pub delta: i64,
    pub side: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KalshiMarketLifecycleEventType {
    Created,
    Activated,
    Deactivated,
    CloseDateUpdated,
    Determined,
    Settled,
}

impl FromStr for KalshiMarketLifecycleEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(KalshiMarketLifecycleEventType::Created),
            "activated" => Ok(KalshiMarketLifecycleEventType::Activated),
            "deactivated" => Ok(KalshiMarketLifecycleEventType::Deactivated),
            "close_date_updated" => Ok(KalshiMarketLifecycleEventType::CloseDateUpdated),
            "determined" => Ok(KalshiMarketLifecycleEventType::Determined),
            "settled" => Ok(KalshiMarketLifecycleEventType::Settled),
            _ => Err(format!("Unknown event type: {}", s)),
        }
    }
}

fn deserialize_event_type<'de, D>(deserializer: D) -> Result<KalshiMarketLifecycleEventType, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    KalshiMarketLifecycleEventType::from_str(&s)
        .map_err(serde::de::Error::custom)
}

impl KalshiMarketLifecycleEventType {
    pub fn to_status(&self, is_deactivated: Option<bool>) -> Option<KalshiMarketStatus> {
        match self {
            Self::Created => Some(KalshiMarketStatus::Unopened),
            Self::Activated => Some(KalshiMarketStatus::Open),
            Self::Deactivated => {
                if is_deactivated == Some(true) {
                    Some(KalshiMarketStatus::Paused)
                } else {
                    Some(KalshiMarketStatus::Open)
                }
            }
            Self::CloseDateUpdated => Some(KalshiMarketStatus::Open),
            Self::Determined => Some(KalshiMarketStatus::Closed),
            Self::Settled => Some(KalshiMarketStatus::Settled),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiMarketLifecycleMsg {
    #[serde(deserialize_with = "deserialize_event_type")]
    pub event_type: KalshiMarketLifecycleEventType,
    pub market_ticker: String,
    #[serde(default)]
    pub open_ts: Option<i64>,
    #[serde(default)]
    pub close_ts: Option<i64>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub determination_ts: Option<i64>,
    #[serde(default)]
    pub settled_ts: Option<i64>,
    #[serde(default)]
    pub is_deactivated: Option<bool>,
    #[serde(default)]
    pub additional_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiTrade {
    pub market_ticker: String,
    pub trade_id: Option<String>,
    pub side: Option<String>,
    pub yes_price: Option<f64>,
    pub no_price: Option<f64>,
    pub count: Option<i64>,
    pub created_time: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KalshiChannel {
    Ticker,
    OrderbookDelta,
    Trade,
    MarketLifecycle,
}

impl KalshiChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            KalshiChannel::Ticker => "ticker",
            KalshiChannel::OrderbookDelta => "orderbook_delta",
            KalshiChannel::Trade => "trade",
            KalshiChannel::MarketLifecycle => "market_lifecycle_v2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KalshiMarketStatus {
    Unopened,
    Open,
    Active,
    Paused,
    Closed,
    Settled
}

impl KalshiMarketStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KalshiMarketStatus::Unopened => "unopened",
            KalshiMarketStatus::Open => "open",
            KalshiMarketStatus::Active => "active",
            KalshiMarketStatus::Paused => "paused",
            KalshiMarketStatus::Closed => "closed",
            KalshiMarketStatus::Settled => "settled",
        }
    }
}