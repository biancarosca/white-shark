use chrono::{DateTime, Utc};
use std::io::Cursor;
use byteorder::{LittleEndian, ReadBytesExt};
use tracing::info;
use crate::{error::Result, exchanges::binance::sbe::{types::{decode_decimal, micros_to_datetime}, utils::read_var_string8}
};

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

    pub fn print_update(&self) {
        let last_price = (self.bid_price * self.ask_qty + self.ask_price * self.bid_qty) / (self.bid_qty + self.ask_qty);
        info!("⚖️ bid = {}, ask = {}, last_price = {:.3}\n", self.bid_price, self.ask_price, last_price);
    }
}