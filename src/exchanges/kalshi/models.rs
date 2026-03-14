use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;

use crate::trader::constants::FILL_OR_KILL_ORDER_PRICE;

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
    #[serde(default, alias = "delta_fp", deserialize_with = "deserialize_delta")]
    pub delta: i64,
    pub side: String,
}

fn deserialize_delta<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    use serde::de;

    struct DeltaVisitor;
    impl<'de> de::Visitor<'de> for DeltaVisitor {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an integer or numeric string")
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> { Ok(v) }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> { Ok(v as i64) }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<i64, E> { Ok(v as i64) }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            v.parse::<f64>().map(|f| f as i64).map_err(de::Error::custom)
        }
    }
    deserializer.deserialize_any(DeltaVisitor)
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
    Fill,
}

impl KalshiChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            KalshiChannel::Ticker => "ticker",
            KalshiChannel::OrderbookDelta => "orderbook_delta",
            KalshiChannel::Trade => "trade",
            KalshiChannel::MarketLifecycle => "market_lifecycle_v2",
            KalshiChannel::Fill => "fill",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KalshiFill {
    pub trade_id: String,
    pub order_id: String,
    pub market_ticker: String,
    pub is_taker: bool,
    pub side: String,
    #[serde(default)]
    pub yes_price: Option<i64>,
    #[serde(default)]
    pub yes_price_dollars: Option<String>,
    #[serde(default)]
    pub count: Option<i64>,
    #[serde(default)]
    pub count_fp: Option<String>,
    #[serde(default)]
    pub fee_cost: Option<String>,
    pub action: String,
    pub ts: i64,
    #[serde(default)]
    pub client_order_id: Option<String>,
    #[serde(default)]
    pub post_position: Option<i64>,
    #[serde(default)]
    pub post_position_fp: Option<String>,
    #[serde(default)]
    pub purchased_side: Option<String>,
    #[serde(default)]
    pub subaccount: Option<i64>,
}

impl KalshiFill {
    fn parse_fp(s: &str) -> i64 {
        s.parse::<f64>().map(|f| f as i64).unwrap_or(0)
    }

    pub fn get_count(&self) -> i64 {
        self.count.unwrap_or_else(|| {
            self.count_fp.as_deref().map(Self::parse_fp).unwrap_or(0)
        })
    }

    pub fn get_post_position(&self) -> i64 {
        self.post_position.unwrap_or_else(|| {
            self.post_position_fp.as_deref().map(Self::parse_fp).unwrap_or(0)
        })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Yes,
    No,
}

impl OrderSide {
    pub fn opposite(self) -> Self {
        match self {
            OrderSide::Yes => OrderSide::No,
            OrderSide::No => OrderSide::Yes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderAction {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    FillOrKill,
    GoodTillCanceled,
    ImmediateOrCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateOrderRequest {
    pub ticker: String,
    pub action: OrderAction,
    pub side: OrderSide,
    pub time_in_force: TimeInForce,
    pub count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_only: Option<bool>,
}

impl CreateOrderRequest {
    pub fn get_base_order(ticker: String, action: OrderAction, side: OrderSide, count: u64, price: u64) -> Self {
        let mut base = Self {
            ticker,
            action,
            side,
            time_in_force: TimeInForce::FillOrKill,
            count,
            yes_price: None,
            no_price: None,
            post_only: None,
        };

        match side {
            OrderSide::Yes => base.yes_price = Some(price),
            OrderSide::No => base.no_price = Some(price),
        }

        base
    }
    pub fn market_order(ticker: String, action: OrderAction, side: OrderSide, count: u64, price: u64) -> Self {
        let base = Self::get_base_order(ticker, action, side, count, price);
        base
    }

    pub fn limit_order(ticker: String, action: OrderAction, side: OrderSide, count: u64, price: u64) -> Self {
        let mut base = Self::get_base_order(ticker, action, side, count, price);
        base.time_in_force = TimeInForce::GoodTillCanceled;
        base.post_only = Some(true);
        base
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateOrderResponse {
    pub order: KalshiOrder,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KalshiOrder {
    pub order_id: String,
    pub user_id: String,
    pub client_order_id: String,
    pub ticker: String,
    pub side: String,
    pub action: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub status: String,
    #[serde(default)]
    pub yes_price: Option<i64>,
    #[serde(default)]
    pub no_price: Option<i64>,
    #[serde(default)]
    pub yes_price_dollars: Option<String>,
    #[serde(default)]
    pub no_price_dollars: Option<String>,
    #[serde(default)]
    pub fill_count: Option<i64>,
    #[serde(default)]
    pub fill_count_fp: Option<String>,
    #[serde(default)]
    pub remaining_count: Option<i64>,
    #[serde(default)]
    pub remaining_count_fp: Option<String>,
    #[serde(default)]
    pub initial_count: Option<i64>,
    #[serde(default)]
    pub initial_count_fp: Option<String>,
    #[serde(default)]
    pub taker_fees: Option<i64>,
    #[serde(default)]
    pub maker_fees: Option<i64>,
    #[serde(default)]
    pub taker_fill_cost: Option<i64>,
    #[serde(default)]
    pub maker_fill_cost: Option<i64>,
    #[serde(default)]
    pub taker_fill_cost_dollars: Option<String>,
    #[serde(default)]
    pub maker_fill_cost_dollars: Option<String>,
    #[serde(default)]
    pub queue_position: Option<i64>,
    #[serde(default)]
    pub taker_fees_dollars: Option<String>,
    #[serde(default)]
    pub maker_fees_dollars: Option<String>,
    #[serde(default)]
    pub expiration_time: Option<String>,
    #[serde(default)]
    pub created_time: Option<String>,
    #[serde(default)]
    pub last_update_time: Option<String>,
    #[serde(default)]
    pub self_trade_prevention_type: Option<String>,
    #[serde(default)]
    pub order_group_id: Option<String>,
    #[serde(default)]
    pub cancel_order_on_pause: bool,
    #[serde(default)]
    pub subaccount_number: Option<i64>,
}

impl KalshiOrder {
    fn parse_fp(s: &str) -> i64 {
        s.parse::<f64>().map(|f| f as i64).unwrap_or(0)
    }

    pub fn get_fill_count(&self) -> i64 {
        self.fill_count.unwrap_or_else(|| {
            self.fill_count_fp.as_deref().map(Self::parse_fp).unwrap_or(0)
        })
    }

    pub fn get_remaining_count(&self) -> i64 {
        self.remaining_count.unwrap_or_else(|| {
            self.remaining_count_fp.as_deref().map(Self::parse_fp).unwrap_or(0)
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetOrdersResponse {
    pub orders: Vec<KalshiOrder>,
    pub cursor: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchCreateOrdersRequest {
    pub orders: Vec<CreateOrderRequest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchCreateOrdersResponse {
    pub orders: Vec<BatchCreateOrderResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchCreateOrderResult {
    #[serde(default)]
    pub client_order_id: Option<String>,
    #[serde(default)]
    pub order: Option<KalshiOrder>,
    #[serde(default)]
    pub error: Option<BatchOrderError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchOrderError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchCancelOrdersRequest {
    pub orders: Vec<KalshiCancelOrder>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KalshiCancelOrder {
    pub order_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KalshiBatchCancelOrdersResponse {
    pub orders: Vec<KalshiBatchCancelOrderResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KalshiBatchCancelOrderResponse {
    pub order_id: String,
    #[serde(default)]
    pub reduced_by: Option<u64>,
    #[serde(default)]
    pub reduced_by_fp: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TickUpdate {
    pub ticker: String,
    pub asset: String,
    pub timestamp: DateTime<Utc>,
    pub yes_ask: f64,
    pub yes_bid: f64,
    pub no_ask: f64,
    pub no_bid: f64,
    pub yes_ask_qty: i64,
    pub no_ask_qty: i64,
    pub close_time: Option<DateTime<Utc>>,
}

impl TickUpdate {
    pub fn from_orderbook(
        ob: &KalshiOrderbook,
        asset: String,
        close_time: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            ticker: ob.market_ticker.clone(),
            asset,
            timestamp: Utc::now(),
            yes_ask: ob.top_yes_ask(),
            yes_bid: ob.top_yes_bid(),
            no_ask: ob.top_no_ask(),
            no_bid: ob.top_no_bid(),
            yes_ask_qty: ob.yes_ask_qty_at_or_above(FILL_OR_KILL_ORDER_PRICE),
            no_ask_qty: ob.no_ask_qty_at_or_above(FILL_OR_KILL_ORDER_PRICE),
            close_time,
        }
    }

    pub fn seconds_until_close(&self) -> Option<i64> {
        self.close_time
            .map(|ct| (ct - self.timestamp).num_seconds())
    }
}