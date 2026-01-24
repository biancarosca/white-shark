use chrono::{DateTime, Utc};
use tracing::info;
use crate::{
    error::Result,
    exchanges::binance::sbe::{
        types::micros_to_datetime,
        utils::SbeCursor,
    },
};

#[derive(Debug, Clone)]
pub struct BestBidAskStreamEvent<'a> {
    pub event_time: DateTime<Utc>,
    pub book_update_id: i64,
    pub bid_price: f64,
    pub bid_qty: f64,
    pub ask_price: f64,
    pub ask_qty: f64,
    pub symbol: &'a str,
}

impl<'a> BestBidAskStreamEvent<'a> {
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut cursor = SbeCursor::new(data);

        let event_time_micros = cursor.read_i64_le()?;
        let book_update_id = cursor.read_i64_le()?;
        let price_exponent = cursor.read_i8()?;
        let qty_exponent = cursor.read_i8()?;
        let price_scale = 10f64.powi(price_exponent as i32);
        let qty_scale = 10f64.powi(qty_exponent as i32);

        let bid_price_mantissa = cursor.read_i64_le()?;
        let bid_price = bid_price_mantissa as f64 * price_scale;

        let bid_qty_mantissa = cursor.read_i64_le()?;
        let bid_qty = bid_qty_mantissa as f64 * qty_scale;

        let ask_price_mantissa = cursor.read_i64_le()?;
        let ask_price = ask_price_mantissa as f64 * price_scale;

        let ask_qty_mantissa = cursor.read_i64_le()?;
        let ask_qty = ask_qty_mantissa as f64 * qty_scale;

        let symbol = cursor.read_var_string8()?;

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