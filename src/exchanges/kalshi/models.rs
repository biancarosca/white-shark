//! Kalshi data models

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// WebSocket Messages
// ============================================================================

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

    pub fn ticker_all(id: u64) -> Self {
        Self::new(id, vec!["ticker".to_string()], None)
    }

    pub fn ticker(id: u64, tickers: Vec<String>) -> Self {
        Self::new(id, vec!["ticker".to_string()], Some(tickers))
    }

    pub fn orderbook(id: u64, tickers: Vec<String>) -> Self {
        Self::new(id, vec!["orderbook_delta".to_string()], Some(tickers))
    }

    pub fn market_status(id: u64) -> Self {
        Self::new(id, vec!["market_lifecycle".to_string()], None)
    }

    pub fn trades(id: u64, tickers: Option<Vec<String>>) -> Self {
        Self::new(id, vec!["trade".to_string()], tickers)
    }

    pub fn multi(id: u64, channels: Vec<String>, tickers: Option<Vec<String>>) -> Self {
        Self::new(id, channels, tickers)
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

    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

// ============================================================================
// Kalshi Events (sent through channels)
// ============================================================================

/// Event emitted by Kalshi WebSocket
#[derive(Debug, Clone)]
pub enum KalshiEvent {
    /// Market status changed (opened, closed, etc.)
    MarketStatusChanged {
        ticker: String,
        old_status: Option<String>,
        new_status: String,
    },
    /// Ticker/price update
    TickerUpdate(KalshiTicker),
    /// Orderbook update
    OrderbookUpdate(KalshiOrderbook),
    /// Trade executed
    Trade(KalshiTrade),
}

// ============================================================================
// Ticker Data
// ============================================================================

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

    pub fn implied_no_bid(&self) -> Option<f64> {
        self.yes_ask_f64().map(|ya| 1.0 - ya)
    }

    /// Check for arbitrage: if YES ask + NO ask < 1.0
    pub fn check_arbitrage(&self) -> Option<f64> {
        if let (Some(yes_ask), Some(yes_bid)) = (self.yes_ask_f64(), self.yes_bid_f64()) {
            let no_ask = 1.0 - yes_bid;
            let total = yes_ask + no_ask;
            if total < 1.0 {
                return Some(total);
            }
        }
        None
    }
}

// ============================================================================
// Market Data (REST)
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiMarket {
    pub ticker: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: String,
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

impl KalshiMarket {
    /// Check if this is a 15-minute crypto market
    pub fn is_15m_crypto(&self) -> bool {
        let ticker_upper = self.ticker.to_uppercase();
        ticker_upper.contains("15M")
            && (ticker_upper.contains("ETH")
                || ticker_upper.contains("BTC")
                || ticker_upper.contains("SOL"))
    }

    /// Extract the base crypto symbol (ETH, BTC, etc.)
    pub fn crypto_symbol(&self) -> Option<&'static str> {
        let ticker_upper = self.ticker.to_uppercase();
        if ticker_upper.contains("ETH") {
            Some("ETH")
        } else if ticker_upper.contains("BTC") {
            Some("BTC")
        } else if ticker_upper.contains("SOL") {
            Some("SOL")
        } else {
            None
        }
    }

    /// Check if market is active
    pub fn is_active(&self) -> bool {
        self.status.to_lowercase() == "active"
    }

    /// Check if market is closed
    pub fn is_closed(&self) -> bool {
        let status = self.status.to_lowercase();
        status == "closed" || status == "determined" || status == "finalized"
    }
}

#[derive(Debug, Deserialize)]
pub struct MarketsResponse {
    pub markets: Vec<KalshiMarket>,
    pub cursor: Option<String>,
}

// ============================================================================
// Market Lifecycle Events
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketLifecycleEvent {
    pub market_ticker: String,
    pub old_status: Option<String>,
    pub new_status: String,
    #[serde(default)]
    pub timestamp: Option<String>,
}

impl MarketLifecycleEvent {
    pub fn is_opening(&self) -> bool {
        self.new_status.to_lowercase() == "active"
            || self.new_status.to_lowercase() == "open"
    }

    pub fn is_closing(&self) -> bool {
        let status = self.new_status.to_lowercase();
        status == "closed" || status == "determined" || status == "finalized"
    }
}

// ============================================================================
// Orderbook Data
// ============================================================================

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

// ============================================================================
// Trade Data
// ============================================================================

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

// ============================================================================
// Kalshi Channels
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KalshiChannel {
    Ticker,
    OrderbookDelta,
    Trade,
    MarketLifecycle,
    Fill,
}

impl KalshiChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            KalshiChannel::Ticker => "ticker",
            KalshiChannel::OrderbookDelta => "orderbook_delta",
            KalshiChannel::Trade => "trade",
            KalshiChannel::MarketLifecycle => "market_lifecycle",
            KalshiChannel::Fill => "fill",
        }
    }
}

impl From<KalshiChannel> for String {
    fn from(channel: KalshiChannel) -> Self {
        channel.as_str().to_string()
    }
}
