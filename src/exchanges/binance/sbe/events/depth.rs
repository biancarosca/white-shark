use chrono::{DateTime, Utc};
use std::io::Cursor;
use byteorder::{LittleEndian, ReadBytesExt};
use tracing::info;
use crate::{error::Result, 
    exchanges::binance::sbe::{types::{decode_decimal, micros_to_datetime}, 
    utils::{read_group_size16, read_var_string8}}
};

#[derive(Debug, Clone)]
pub struct DepthLevel {
    pub price: f64,
    pub qty: f64,
}

impl DepthLevel {
    pub fn decode(cursor: &mut Cursor<&[u8]>, price_exponent: i8, qty_exponent: i8) -> Result<Self> {
        let price_mantissa = cursor.read_i64::<LittleEndian>()?;
        let price = decode_decimal(price_mantissa, price_exponent);
        
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

        let event_time_micros = cursor.read_i64::<LittleEndian>()?;
        
        let book_update_id = cursor.read_i64::<LittleEndian>()?;
        
        let price_exponent = cursor.read_i8()?;
        
        let qty_exponent = cursor.read_i8()?;
        
        let (_bids_block_length, num_bids) = read_group_size16(&mut cursor)?;
        let mut bids = Vec::with_capacity(num_bids as usize);
        for _ in 0..num_bids {
            bids.push(DepthLevel::decode(&mut cursor, price_exponent, qty_exponent)?);
        }
        
        let (_asks_block_length, num_asks) = read_group_size16(&mut cursor)?;
        let mut asks = Vec::with_capacity(num_asks as usize);
        for _ in 0..num_asks {
            asks.push(DepthLevel::decode(&mut cursor, price_exponent, qty_exponent)?);
        }
        
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
        if imbalance_top_5 > 100.0 {
            info!("ALERT: N_5: imbalance\n");
        }
        if imbalance_top_10 > 100.0 {
            info!("ALERT: N_10: imbalance\n");
        }
        if imbalance_all > 100.0 {
            info!("ALERT: All: imbalance\n");
        }
    }
}