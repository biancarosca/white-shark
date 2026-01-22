use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::exchanges::{OrderbookUpdate, PriceLevel, PriceUpdate, TradeSide};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinanceStream {
    Trade,
    BestBidAsk,
    Depth,
    Depth20,
    Kline(KlineInterval),
}

impl BinanceStream {
    pub fn stream_name(&self, symbol: &str) -> String {
        let symbol_lower = symbol.to_lowercase();
        match self {
            BinanceStream::Trade => format!("{}@trade", symbol_lower),
            BinanceStream::BestBidAsk => format!("{}@bestBidAsk", symbol_lower),
            BinanceStream::Depth => format!("{}@depth", symbol_lower),
            BinanceStream::Depth20 => format!("{}@depth20", symbol_lower),
            BinanceStream::Kline(interval) => {
                format!("{}@kline_{}", symbol_lower, interval.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlineInterval {
    M1,
    M3,
    M5,
    M15,
    M30,
    H1,
    H2,
    H4,
    H6,
    H8,
    H12,
    D1,
    D3,
    W1,
    Month1,
}

impl KlineInterval {
    pub fn as_str(&self) -> &'static str {
        match self {
            KlineInterval::M1 => "1m",
            KlineInterval::M3 => "3m",
            KlineInterval::M5 => "5m",
            KlineInterval::M15 => "15m",
            KlineInterval::M30 => "30m",
            KlineInterval::H1 => "1h",
            KlineInterval::H2 => "2h",
            KlineInterval::H4 => "4h",
            KlineInterval::H6 => "6h",
            KlineInterval::H8 => "8h",
            KlineInterval::H12 => "12h",
            KlineInterval::D1 => "1d",
            KlineInterval::D3 => "3d",
            KlineInterval::W1 => "1w",
            KlineInterval::Month1 => "1M",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SubscribeRequest {
    pub method: String,
    pub params: Vec<String>,
    pub id: u64,
}

impl SubscribeRequest {
    pub fn subscribe(id: u64, streams: Vec<String>) -> Self {
        Self {
            method: "SUBSCRIBE".to_string(),
            params: streams,
            id,
        }
    }

    pub fn unsubscribe(id: u64, streams: Vec<String>) -> Self {
        Self {
            method: "UNSUBSCRIBE".to_string(),
            params: streams,
            id,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinanceTrade {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "t")]
    pub trade_id: u64,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub quantity: String,
    #[serde(rename = "T")]
    pub trade_time: u64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

impl BinanceTrade {
    pub fn to_price_update(&self) -> PriceUpdate {
        let price = self.price.parse::<f64>().unwrap_or(0.0);
        let timestamp = DateTime::from_timestamp_millis(self.event_time as i64)
            .unwrap_or_else(Utc::now);

        PriceUpdate {
            exchange: "binance".to_string(),
            symbol: self.symbol.clone(),
            timestamp,
            bid: None,
            ask: None,
            last_price: Some(price),
            volume_24h: None,
        }
    }

    pub fn side(&self) -> TradeSide {
        if self.is_buyer_maker {
            TradeSide::Sell
        } else {
            TradeSide::Buy
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinanceBestBidAsk {
    #[serde(rename = "e")]
    pub event_type: Option<String>,
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub best_bid_price: String,
    #[serde(rename = "B")]
    pub best_bid_qty: String,
    #[serde(rename = "a")]
    pub best_ask_price: String,
    #[serde(rename = "A")]
    pub best_ask_qty: String,
}

impl BinanceBestBidAsk {
    pub fn to_price_update(&self) -> PriceUpdate {
        let bid = self.best_bid_price.parse::<f64>().ok();
        let ask = self.best_ask_price.parse::<f64>().ok();

        PriceUpdate {
            exchange: "binance".to_string(),
            symbol: self.symbol.clone(),
            timestamp: Utc::now(),
            bid,
            ask,
            last_price: None,
            volume_24h: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinanceDepthUpdate {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub final_update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<[String; 2]>,
    #[serde(rename = "a")]
    pub asks: Vec<[String; 2]>,
}

impl BinanceDepthUpdate {
    pub fn to_orderbook_update(&self) -> OrderbookUpdate {
        let timestamp = DateTime::from_timestamp_millis(self.event_time as i64)
            .unwrap_or_else(Utc::now);

        let bids = self
            .bids
            .iter()
            .filter_map(|[price, qty]| {
                Some(PriceLevel {
                    price: price.parse().ok()?,
                    quantity: qty.parse().ok()?,
                })
            })
            .collect();

        let asks = self
            .asks
            .iter()
            .filter_map(|[price, qty]| {
                Some(PriceLevel {
                    price: price.parse().ok()?,
                    quantity: qty.parse().ok()?,
                })
            })
            .collect();

        OrderbookUpdate {
            symbol: self.symbol.clone(),
            timestamp,
            bids,
            asks,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinancePartialDepth {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinanceKlineEvent {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: BinanceKline,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinanceKline {
    #[serde(rename = "t")]
    pub open_time: u64,
    #[serde(rename = "T")]
    pub close_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "i")]
    pub interval: String,
    #[serde(rename = "f")]
    pub first_trade_id: i64,
    #[serde(rename = "L")]
    pub last_trade_id: i64,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "n")]
    pub number_of_trades: u64,
    #[serde(rename = "x")]
    pub is_closed: bool,
    #[serde(rename = "q")]
    pub quote_asset_volume: String,
    #[serde(rename = "V")]
    pub taker_buy_base_volume: String,
    #[serde(rename = "Q")]
    pub taker_buy_quote_volume: String,
}

impl BinanceKline {
    pub fn ohlcv(&self) -> Option<(f64, f64, f64, f64, f64)> {
        Some((
            self.open.parse().ok()?,
            self.high.parse().ok()?,
            self.low.parse().ok()?,
            self.close.parse().ok()?,
            self.volume.parse().ok()?,
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionResponse {
    pub result: Option<serde_json::Value>,
    pub id: u64,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamMessage {
    pub stream: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum BinanceMessage {
    SubscriptionResponse(SubscriptionResponse),
    StreamMessage(StreamMessage),
}

#[derive(Debug, Clone)]
pub enum BinanceStreamData {
    Trade(BinanceTrade),
    BestBidAsk(BinanceBestBidAsk),
    DepthUpdate(BinanceDepthUpdate),
    PartialDepth(BinancePartialDepth),
    Kline(BinanceKlineEvent),
    Unknown(serde_json::Value),
}

impl BinanceMessage {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        // Try to parse as subscription response first (has "id" field)
        if let Ok(response) = serde_json::from_str::<SubscriptionResponse>(text) {
            return Ok(BinanceMessage::SubscriptionResponse(response));
        }
        
        // Otherwise try to parse as stream message
        let stream_msg = serde_json::from_str::<StreamMessage>(text)?;
        Ok(BinanceMessage::StreamMessage(stream_msg))
    }
}

impl StreamMessage {
    pub fn parse_data(&self) -> BinanceStreamData {
        if self.stream.contains("@trade") {
            if let Ok(trade) = serde_json::from_value(self.data.clone()) {
                return BinanceStreamData::Trade(trade);
            }
        } else if self.stream.contains("@bestBidAsk") {
            if let Ok(bba) = serde_json::from_value(self.data.clone()) {
                return BinanceStreamData::BestBidAsk(bba);
            }
        } else if self.stream.contains("@depth20") {
            if let Ok(depth) = serde_json::from_value(self.data.clone()) {
                return BinanceStreamData::PartialDepth(depth);
            }
        } else if self.stream.contains("@depth") {
            if let Ok(depth) = serde_json::from_value(self.data.clone()) {
                return BinanceStreamData::DepthUpdate(depth);
            }
        } else if self.stream.contains("@kline") {
            if let Ok(kline) = serde_json::from_value(self.data.clone()) {
                return BinanceStreamData::Kline(kline);
            }
        }

        BinanceStreamData::Unknown(self.data.clone())
    }
}

