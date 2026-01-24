use chrono::{DateTime, Utc};
use tracing::info;
use crate::{
    Error,
    error::Result,
    exchanges::binance::sbe::{
        types::micros_to_datetime,
        utils::{read_group_size, SbeCursor},
    },
};

#[derive(Debug, Clone)]
pub struct Trade {
    pub id: i64,
    pub price: f64,
    pub qty: f64,
    pub is_buyer_maker: bool,
}

#[derive(Debug, Clone)]
pub struct TradeStreamEvent<'a> {
    pub event_time: DateTime<Utc>,
    pub transact_time: DateTime<Utc>,
    pub last_trade: Option<Trade>,
    pub symbol: &'a str,
}

impl<'a> TradeStreamEvent<'a> {
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut cursor = SbeCursor::new(data);

        let event_time_micros = cursor.read_i64_le()?;
        let transact_time_micros = cursor.read_i64_le()?;
        let price_exponent = cursor.read_i8()?;
        let qty_exponent = cursor.read_i8()?;
        let price_scale = 10f64.powi(price_exponent as i32);
        let qty_scale = 10f64.powi(qty_exponent as i32);

        let (block_length, num_trades) = read_group_size(&mut cursor)?;
        let block_length = block_length as usize;

        let last_trade = if num_trades > 0 {
            if num_trades > 1 {
                let skip_bytes = (num_trades - 1) as usize * block_length;
                cursor.skip(skip_bytes)?;
            }

            let position_before = cursor.position();
            if cursor.remaining() < block_length {
                return Err(Error::SbeDecode(format!(
                    "Not enough data for last trade: need {} bytes, have {} bytes",
                    block_length,
                    cursor.remaining()
                )));
            }

            if block_length < 25 {
                return Err(Error::SbeDecode(format!(
                    "Trade block too short: need at least 25 bytes, have {} bytes",
                    block_length
                )));
            }

            let id = cursor.read_i64_le()?;
            let price_mantissa = cursor.read_i64_le()?;
            let price = price_mantissa as f64 * price_scale;
            let qty_mantissa = cursor.read_i64_le()?;
            let qty = qty_mantissa as f64 * qty_scale;
            let is_buyer_maker = cursor.read_u8()? != 0;

            let bytes_read = cursor.position() - position_before;
            if bytes_read < block_length {
                cursor.skip(block_length - bytes_read)?;
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

        let symbol = cursor.read_var_string8()?;

        Ok(Self {
            event_time: micros_to_datetime(event_time_micros as u64),
            transact_time: micros_to_datetime(transact_time_micros as u64),
            last_trade,
            symbol,
        })
    }

    pub fn print_update(&self) {
        let last_price = self.last_trade.as_ref().map(|t| t.price).unwrap_or(0.0);
        info!("⚡ price = {}\n", last_price);
    }
}
