use chrono::{DateTime, Utc};
use std::io::Cursor;
use byteorder::{LittleEndian, ReadBytesExt};
use tracing::info;
use crate::{Error, error::Result, exchanges::binance::sbe::{types::{decode_decimal, micros_to_datetime}, 
    utils::{read_group_size, read_var_string8}}
};

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
        
        let (block_length, num_trades) = read_group_size(&mut cursor)?;
        

        let last_trade = if num_trades > 0 {
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
            
            let id = cursor.read_i64::<LittleEndian>()?;
            
            let price_mantissa = cursor.read_i64::<LittleEndian>()?;
            let price = decode_decimal(price_mantissa, price_exponent);
            
            let qty_mantissa = cursor.read_i64::<LittleEndian>()?;
            let qty = decode_decimal(qty_mantissa, qty_exponent);
            
            let is_buyer_maker = cursor.read_u8()? != 0;
            
            let bytes_so_far = cursor.position() as usize - position_before;
            let remaining_in_block = block_length as usize - bytes_so_far;
            
            if remaining_in_block >= 1 {
                let _is_best_match = cursor.read_u8()?;
            }
            
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

    pub fn print_update(&self) {
        let last_price = self.trades.last().map(|t| t.price).unwrap_or(0.0);
        info!("⚡ price = {}\n", last_price);
    }
}
