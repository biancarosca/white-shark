use std::io::{Cursor, Read};

use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use tracing::info;

use crate::error::{Error, Result};
use crate::exchanges::PriceUpdate;

use super::types::*;

// Helper to read variable-length string (varString8)
fn read_var_string8(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let position = cursor.position() as usize;
    let data = cursor.get_ref();
    
    if position >= data.len() {
        return Err(Error::SbeDecode("Not enough data to read string length".into()));
    }
    
    let length = data[position] as usize;
    
    if position + 1 + length > data.len() {
        return Err(Error::SbeDecode(format!(
            "Not enough data to read string: need {} bytes, have {} bytes",
            length + 1,
            data.len() - position
        )));
    }
    
    cursor.read_u8()?; // Skip the length byte we already read
    let mut bytes = vec![0u8; length];
    cursor.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|e| Error::SbeDecode(format!("Invalid UTF-8 in symbol: {}", e)))
}

// Helper to read repeating group with groupSizeEncoding (blockLength: u16, numInGroup: u32)
fn read_group_size(cursor: &mut Cursor<&[u8]>) -> Result<(u16, u32)> {
    let block_length = cursor.read_u16::<LittleEndian>()?;
    let num_in_group = cursor.read_u32::<LittleEndian>()?;
    Ok((block_length, num_in_group))
}

// Helper to read repeating group with groupSize16Encoding (blockLength: u16, numInGroup: u16)
fn read_group_size16(cursor: &mut Cursor<&[u8]>) -> Result<(u16, u16)> {
    let block_length = cursor.read_u16::<LittleEndian>()?;
    let num_in_group = cursor.read_u16::<LittleEndian>()?;
    Ok((block_length, num_in_group))
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub id: i64,
    pub price: f64,
    pub qty: f64,
    pub is_buyer_maker: bool,
}

#[derive(Debug, Clone)]
pub struct TradeStreamEvent {
    pub event_time: DateTime<Utc>,
    pub transact_time: DateTime<Utc>,
    pub trades: Vec<Trade>,
    pub symbol: String,
}

impl TradeStreamEvent {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);
        let data_len = data.len();

        let event_time_micros = cursor.read_i64::<LittleEndian>()?;
        let transact_time_micros = cursor.read_i64::<LittleEndian>()?;
        let price_exponent = cursor.read_i8()?;
        let qty_exponent = cursor.read_i8()?;
        
        // trades group (groupSizeEncoding) = 2 bytes (block_length) + 4 bytes (numInGroup) = 6 bytes
        let (block_length, num_trades) = read_group_size(&mut cursor)?;
        

        // We only need the last trade's price (most recent) for last_price
        // If there's only 1 trade, we'll parse it. If multiple, we'll skip to the last one.
        let last_trade = if num_trades > 0 {
            // If multiple trades, skip to the last one (most recent price)
            if num_trades > 1 {
                let skip_bytes = (num_trades - 1) as usize * block_length as usize;
                let current_pos = cursor.position() as usize;
                if current_pos + skip_bytes <= data_len {
                    cursor.set_position((current_pos + skip_bytes) as u64);
                } else {
                    return Err(Error::SbeDecode(format!(
                        "Not enough data to skip to last trade: need {} bytes, have {} bytes",
                        skip_bytes, data_len - current_pos
                    )));
                }
            }
            
            let position_before = cursor.position() as usize;
            let remaining = data_len - position_before;
            
            if remaining < block_length as usize {
                return Err(Error::SbeDecode(format!(
                    "Not enough data for last trade: need {} bytes, have {} bytes",
                    block_length, remaining
                )));
            }
            
            // Parse the last trade entry (most recent)
            // id (i64) = 8 bytes
            let id = cursor.read_i64::<LittleEndian>()?;
            
            // price (mantissa64 with priceExponent) = 8 bytes
            let price_mantissa = cursor.read_i64::<LittleEndian>()?;
            let price = decode_decimal(price_mantissa, price_exponent);
            
            // qty (mantissa64 with qtyExponent) = 8 bytes
            let qty_mantissa = cursor.read_i64::<LittleEndian>()?;
            let qty = decode_decimal(qty_mantissa, qty_exponent);
            
            // isBuyerMaker (boolEnum = u8) = 1 byte
            let is_buyer_maker = cursor.read_u8()? != 0;
            
            // isBestMatch (boolEnum, constant True) = 1 byte (may be omitted)
            let bytes_so_far = cursor.position() as usize - position_before;
            let remaining_in_block = block_length as usize - bytes_so_far;
            
            if remaining_in_block >= 1 {
                let _is_best_match = cursor.read_u8()?;
            }
            
            // Skip any remaining padding to reach block_length
            let position_after = cursor.position() as usize;
            let bytes_read = position_after - position_before;
            
            if bytes_read < block_length as usize {
                cursor.set_position((position_before + block_length as usize) as u64);
            }
            
            Some(Trade {
                id,
                price,
                qty,
                is_buyer_maker,
            })
        } else {
            None
        };
        
        let trades = last_trade.into_iter().collect();
        
        // symbol (varString8) - check we have enough data
        let remaining = data_len - cursor.position() as usize;
        if remaining < 1 {
            return Err(Error::SbeDecode(format!(
                "Not enough data for symbol: need at least 1 byte, have {} bytes",
                remaining
            )));
        }
        
        let symbol = read_var_string8(&mut cursor)?;

        Ok(Self {
            event_time: micros_to_datetime(event_time_micros as u64),
            transact_time: micros_to_datetime(transact_time_micros as u64),
            trades,
            symbol,
        })
    }

    // pub fn to_price_update(&self) -> PriceUpdate {
    //     // Use the last trade's price (most recent) as last_price
    //     let last_price = self.trades.last().map(|t| t.price);
        
    //     PriceUpdate {
    //         exchange: "binance".to_string(),
    //         symbol: self.symbol.clone(),
    //         timestamp: self.event_time,
    //         bid: None,
    //         ask: None,
    //         last_price,
    //         volume_24h: None,
    //     }
    // }

    pub fn print_update(&self) {
        let last_price = self.trades.last().map(|t| t.price).unwrap_or(0.0);
        info!("⚡ price = {}\n", last_price);
    }
}

#[derive(Debug, Clone)]
pub struct BestBidAskStreamEvent {
    pub event_time: DateTime<Utc>,
    pub book_update_id: i64,
    pub bid_price: f64,
    pub bid_qty: f64,
    pub ask_price: f64,
    pub ask_qty: f64,
    pub symbol: String,
}

impl BestBidAskStreamEvent {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        let event_time_micros = cursor.read_i64::<LittleEndian>()?;
        let book_update_id = cursor.read_i64::<LittleEndian>()?;
        let price_exponent = cursor.read_i8()?;
        let qty_exponent = cursor.read_i8()?;
        
        let bid_price_mantissa = cursor.read_i64::<LittleEndian>()?;
        let bid_price = decode_decimal(bid_price_mantissa, price_exponent);
        
        let bid_qty_mantissa = cursor.read_i64::<LittleEndian>()?;
        let bid_qty = decode_decimal(bid_qty_mantissa, qty_exponent);
        
        let ask_price_mantissa = cursor.read_i64::<LittleEndian>()?;
        let ask_price = decode_decimal(ask_price_mantissa, price_exponent);
        
        let ask_qty_mantissa = cursor.read_i64::<LittleEndian>()?;
        let ask_qty = decode_decimal(ask_qty_mantissa, qty_exponent);
        
        let symbol = read_var_string8(&mut cursor)?;

        Ok(Self {
            event_time: micros_to_datetime(event_time_micros as u64),
            book_update_id,
            bid_price,
            bid_qty,
            ask_price,
            ask_qty,
            symbol,
        })
    }

    // pub fn to_price_update(&self) -> PriceUpdate {
    //     let last_price = (self.bid_price * self.bid_qty + self.ask_price * self.ask_qty) / (self.bid_qty + self.ask_qty);
    //     PriceUpdate {
    //         exchange: "binance".to_string(),
    //         symbol: self.symbol.clone(),
    //         timestamp: self.event_time,
    //         bid: Some(self.bid_price),
    //         ask: Some(self.ask_price),
    //         last_price: Some(last_price),
    //         volume_24h: None,
    //     }
    // }

    pub fn print_update(&self) {
        let last_price = (self.bid_price * self.ask_qty + self.ask_price * self.bid_qty) / (self.bid_qty + self.ask_qty);
        info!("⚖️ bid = {}, ask = {}, last_price = {:.3}\n", self.bid_price, self.ask_price, last_price);
    }
}

#[derive(Debug, Clone)]
pub struct DepthLevel {
    pub price: f64,
    pub qty: f64,
}

impl DepthLevel {
    pub fn decode(cursor: &mut Cursor<&[u8]>, price_exponent: i8, qty_exponent: i8) -> Result<Self> {
        // price (mantissa64 with priceExponent)
        let price_mantissa = cursor.read_i64::<LittleEndian>()?;
        let price = decode_decimal(price_mantissa, price_exponent);
        
        // qty (mantissa64 with qtyExponent)
        let qty_mantissa = cursor.read_i64::<LittleEndian>()?;
        let qty = decode_decimal(qty_mantissa, qty_exponent);

        Ok(Self { price, qty })
    }
}

#[derive(Debug, Clone)]
pub struct DepthSnapshotStreamEvent {
    pub event_time: DateTime<Utc>,
    pub book_update_id: i64,
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
    pub symbol: String,
}

impl DepthSnapshotStreamEvent {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        // eventTime (i64)
        let event_time_micros = cursor.read_i64::<LittleEndian>()?;
        
        // bookUpdateId (i64)
        let book_update_id = cursor.read_i64::<LittleEndian>()?;
        
        // priceExponent (i8)
        let price_exponent = cursor.read_i8()?;
        
        // qtyExponent (i8)
        let qty_exponent = cursor.read_i8()?;
        
        // bids group (groupSize16Encoding)
        let (_bids_block_length, num_bids) = read_group_size16(&mut cursor)?;
        let mut bids = Vec::with_capacity(num_bids as usize);
        for _ in 0..num_bids {
            bids.push(DepthLevel::decode(&mut cursor, price_exponent, qty_exponent)?);
        }
        
        // asks group (groupSize16Encoding)
        let (_asks_block_length, num_asks) = read_group_size16(&mut cursor)?;
        let mut asks = Vec::with_capacity(num_asks as usize);
        for _ in 0..num_asks {
            asks.push(DepthLevel::decode(&mut cursor, price_exponent, qty_exponent)?);
        }
        
        // symbol (varString8)
        let symbol = read_var_string8(&mut cursor)?;

        Ok(Self {
            event_time: micros_to_datetime(event_time_micros as u64),
            book_update_id,
            bids,
            asks,
            symbol,
        })
    }

    pub fn print_update(&self) {
        let top_5_bids_total_qty = self.bids.iter().take(5).map(|b| b.qty).sum::<f64>();
        let top_5_asks_total_qty = self.asks.iter().take(5).map(|a| a.qty).sum::<f64>();

        if top_5_asks_total_qty < 0.0 {
            return;
        }

        let imbalance_top_5 = top_5_bids_total_qty / top_5_asks_total_qty;

        let top_10_bids_total_qty = self.bids.iter().take(10).map(|b| b.qty).sum::<f64>();
        let top_10_asks_total_qty = self.asks.iter().take(10).map(|a| a.qty).sum::<f64>();
        let imbalance_top_10 = top_10_bids_total_qty / top_10_asks_total_qty;

        let all_bids_total_qty = self.bids.iter().map(|b| b.qty).sum::<f64>();
        let all_asks_total_qty = self.asks.iter().map(|a| a.qty).sum::<f64>();
        let imbalance_all = all_bids_total_qty / all_asks_total_qty;

        info!("📕 N_5: bids = {:.2}, asks = {:.2}, ratio = {:.3}", top_5_bids_total_qty, top_5_asks_total_qty, imbalance_top_5);
        info!("📘 N_10: bids = {:.2}, asks = {:.2}, ratio = {:.3}", top_10_bids_total_qty, top_10_asks_total_qty, imbalance_top_10);
        info!("📙 All: bids = {:.2}, asks = {:.2}, ratio = {:.3}\n", all_bids_total_qty, all_asks_total_qty, imbalance_all);
    }
}

#[derive(Debug, Clone)]
pub struct DepthDiffStreamEvent {
    pub event_time: DateTime<Utc>,
    pub first_book_update_id: i64,
    pub last_book_update_id: i64,
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
    pub symbol: String,
}

impl DepthDiffStreamEvent {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        // eventTime (i64)
        let event_time_micros = cursor.read_i64::<LittleEndian>()?;
        
        // firstBookUpdateId (i64)
        let first_book_update_id = cursor.read_i64::<LittleEndian>()?;
        
        // lastBookUpdateId (i64)
        let last_book_update_id = cursor.read_i64::<LittleEndian>()?;
        
        // priceExponent (i8)
        let price_exponent = cursor.read_i8()?;
        
        // qtyExponent (i8)
        let qty_exponent = cursor.read_i8()?;
        
        // bids group (groupSize16Encoding)
        let (_bids_block_length, num_bids) = read_group_size16(&mut cursor)?;
        let mut bids = Vec::with_capacity(num_bids as usize);
        for _ in 0..num_bids {
            bids.push(DepthLevel::decode(&mut cursor, price_exponent, qty_exponent)?);
        }
        
        // asks group (groupSize16Encoding)
        let (_asks_block_length, num_asks) = read_group_size16(&mut cursor)?;
        let mut asks = Vec::with_capacity(num_asks as usize);
        for _ in 0..num_asks {
            asks.push(DepthLevel::decode(&mut cursor, price_exponent, qty_exponent)?);
        }
        
        // symbol (varString8)
        let symbol = read_var_string8(&mut cursor)?;

        Ok(Self {
            event_time: micros_to_datetime(event_time_micros as u64),
            first_book_update_id,
            last_book_update_id,
            bids,
            asks,
            symbol,
        })
    }
}

#[derive(Debug, Clone)]
pub enum SbeMessage {
    Trade(TradeStreamEvent),
    BestBidAsk(BestBidAskStreamEvent),
    DepthDiff(DepthDiffStreamEvent),
    DepthSnapshot(DepthSnapshotStreamEvent),
}

impl SbeMessage {
    pub fn print_update(&self) {
        match self {
            SbeMessage::Trade(e) => e.print_update(),
            SbeMessage::BestBidAsk(e) => e.print_update(),
            SbeMessage::DepthDiff(e) => {
                ()
            },
            SbeMessage::DepthSnapshot(e) => e.print_update(),
            // SbeMessage::DepthDiff(e) => PriceUpdate {
            //     exchange: "binance".to_string(),
            //     symbol: e.symbol.clone(),
            //     timestamp: e.event_time,
            //     bid: e.bids.first().map(|b| b.price),
            //     ask: e.asks.first().map(|a| a.price),
            //     last_price: None,
            //     volume_24h: None,
            // },
            // SbeMessage::DepthSnapshot(e) => PriceUpdate {
            //     exchange: "binance".to_string(),
            //     symbol: e.symbol.clone(),
            //     timestamp: e.event_time,
            //     bid: e.bids.first().map(|b| b.price),
            //     ask: e.asks.first().map(|a| a.price),
            //     last_price: None,
            //     volume_24h: None,
            // },
        }
    }

    pub fn symbol(&self) -> &str {
        match self {
            SbeMessage::Trade(e) => &e.symbol,
            SbeMessage::BestBidAsk(e) => &e.symbol,
            SbeMessage::DepthDiff(e) => &e.symbol,
            SbeMessage::DepthSnapshot(e) => &e.symbol,
        }
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            SbeMessage::Trade(e) => e.event_time,
            SbeMessage::BestBidAsk(e) => e.event_time,
            SbeMessage::DepthDiff(e) => e.event_time,
            SbeMessage::DepthSnapshot(e) => e.event_time,
        }
    }
}
