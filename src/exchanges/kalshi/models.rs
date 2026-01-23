use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
pub struct UnsubscribeMessage {
    pub id: u64,
    pub cmd: String,
    pub params: SubscribeParams,
}

impl UnsubscribeMessage {
    pub fn new(id: u64, channels: Vec<String>, market_tickers: Option<Vec<String>>) -> Self {
        Self {
            id,
            cmd: "unsubscribe".to_string(),
            params: SubscribeParams {
                channels,
                market_tickers,
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
        self.status.as_deref() == Some("subscribed")
    }

}


#[derive(Debug, Clone)]
pub enum KalshiEvent {
    MarketStatusChanged {
        ticker: String,
        old_status: Option<String>,
        new_status: String,
    },
    TickerUpdate(KalshiTicker),
    OrderbookUpdate(KalshiOrderbook),
    Trade(KalshiTrade),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiTicker {
    pub market_ticker: String,
    #[serde(default)]
    pub yes_bid: Option<f64>,
    #[serde(default)]
    pub yes_ask: Option<f64>,
    #[serde(default)]
    pub yes_bid_dollars: Option<String>,
    #[serde(default)]
    pub yes_ask_dollars: Option<String>,
    #[serde(default)]
    pub no_bid: Option<f64>,
    #[serde(default)]
    pub no_ask: Option<f64>,
    #[serde(default)]
    pub last_price: Option<f64>,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub dollar_volume: Option<f64>,
    #[serde(default)]
    pub open_interest: Option<f64>,
    #[serde(default)]
    pub dollar_open_interest: Option<f64>,
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
}

impl KalshiTicker {
    pub fn yes_ask_f64(&self) -> Option<f64> {
        self.yes_ask.or_else(|| {
            self.yes_ask_dollars
                .as_ref()
                .and_then(|s| s.parse::<f64>().ok())
        })
    }

    pub fn yes_bid_f64(&self) -> Option<f64> {
        self.yes_bid.or_else(|| {
            self.yes_bid_dollars
                .as_ref()
                .and_then(|s| s.parse::<f64>().ok())
        })
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
            KalshiChannel::MarketLifecycle => "market_lifecycle",
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