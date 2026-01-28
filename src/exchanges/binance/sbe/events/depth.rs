use chrono::{DateTime, Utc};
use tracing::{info, warn};
use crate::{
    Error,
    error::Result,
    exchanges::binance::sbe::{
        types::micros_to_datetime,
        utils::{read_group_size16, read_i64_le_from, SbeCursor},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct DepthLevels<'a> {
    data: &'a [u8],
    count: u16,
    block_length: u16,
    qty_scale: f64,
}

impl<'a> DepthLevels<'a> {
    fn new(
        data: &'a [u8],
        count: u16,
        block_length: u16,
        qty_scale: f64,
    ) -> Result<Self> {
        if block_length < 16 {
            return Err(Error::SbeDecode(format!(
                "Depth level block too short: need at least 16 bytes, have {} bytes",
                block_length
            )));
        }
        Ok(Self {
            data,
            count,
            block_length,
            qty_scale,
        })
    }

    pub fn sum_qtys_top5_top10_all(&self) -> Result<(f64, f64, f64)> {
        let mut top_5_sum = 0.0_f64;
        let mut top_10_sum = 0.0_f64;
        let mut all_sum = 0.0_f64;
        let block_length = self.block_length as usize;

        let mut offset = 0usize;
        for idx in 0..self.count as usize {
            let qty_offset = offset + 8;
            if qty_offset + 8 > self.data.len() {
                return Err(Error::SbeDecode(format!(
                    "Not enough data for depth level qty: need {} bytes, have {} bytes",
                    qty_offset + 8,
                    self.data.len()
                )));
            }

            let qty_mantissa = read_i64_le_from(&self.data[qty_offset..])?;
            let qty = qty_mantissa as f64 * self.qty_scale;
            if idx < 5 {
                top_5_sum += qty;
            }
            if idx < 10 {
                top_10_sum += qty;
            }
            all_sum += qty;
            offset += block_length;
        }

        Ok((top_5_sum, top_10_sum, all_sum))
    }
}

#[derive(Debug, Clone)]
pub struct DepthSnapshotStreamEvent<'a> {
    pub event_time: DateTime<Utc>,
    pub book_update_id: i64,
    pub bids: DepthLevels<'a>,
    pub asks: DepthLevels<'a>,
    pub symbol: &'a str,
}

impl<'a> DepthSnapshotStreamEvent<'a> {
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut cursor = SbeCursor::new(data);

        let event_time_micros = cursor.read_i64_le()?;
        let book_update_id = cursor.read_i64_le()?;
        let _price_exponent = cursor.read_i8()?;
        let qty_exponent = cursor.read_i8()?;
        let qty_scale = 10f64.powi(qty_exponent as i32);

        let (bids_block_length, num_bids) = read_group_size16(&mut cursor)?;
        let bids_bytes = bids_block_length as usize * num_bids as usize;
        let bids_data = cursor.read_bytes(bids_bytes)?;
        let bids = DepthLevels::new(bids_data, num_bids, bids_block_length, qty_scale)?;

        let (asks_block_length, num_asks) = read_group_size16(&mut cursor)?;
        let asks_bytes = asks_block_length as usize * num_asks as usize;
        let asks_data = cursor.read_bytes(asks_bytes)?;
        let asks = DepthLevels::new(asks_data, num_asks, asks_block_length, qty_scale)?;

        let symbol = cursor.read_var_string8()?;

        Ok(Self {
            event_time: micros_to_datetime(event_time_micros as u64),
            book_update_id,
            bids,
            asks,
            symbol,
        })
    }

    pub fn print_update(&self, imbalance_tx: Option<&tokio::sync::mpsc::Sender<crate::event_processor::ImbalanceAlert>>) {
        let (top_5_bids_total_qty, top_10_bids_total_qty, all_bids_total_qty) =
            match self.bids.sum_qtys_top5_top10_all() {
                Ok(values) => values,
                Err(e) => {
                    warn!("Failed to compute bid quantities: {}", e);
                    return;
                }
            };
        let (top_5_asks_total_qty, top_10_asks_total_qty, all_asks_total_qty) =
            match self.asks.sum_qtys_top5_top10_all() {
                Ok(values) => values,
                Err(e) => {
                    warn!("Failed to compute ask quantities: {}", e);
                    return;
                }
            };

        if top_5_asks_total_qty <= 0.0 {
            return;
        }

        let imbalance_top_5 = top_5_bids_total_qty / top_5_asks_total_qty;
        let imbalance_top_10 = top_10_bids_total_qty / top_10_asks_total_qty;
        let imbalance_all = all_bids_total_qty / all_asks_total_qty;

        info!("imbalance_top_5: {}, imbalance_top_10: {}, imbalance_all: {}", imbalance_top_5, imbalance_top_10, imbalance_all);

        let now = chrono::Utc::now();
        let message_received_time = self.event_time;
        let imbalance_detected_time = now;

        if imbalance_top_5 > 100.0 || imbalance_top_10 > 100.0 || imbalance_all > 100.0 || imbalance_top_5 < 0.01
            || imbalance_top_10 < 0.01 || imbalance_all < 0.01 {
            if let Some(tx) = imbalance_tx {
                let _ = tx.try_send(crate::event_processor::ImbalanceAlert {
                    message_received_time,
                    imbalance_detected_time,
                    symbol: self.symbol.to_string(),
                    imbalance_top_5,
                    imbalance_top_10,
                    imbalance_all,
                    top_5_bids: top_5_bids_total_qty,
                    top_5_asks: top_5_asks_total_qty,
                    top_10_bids: top_10_bids_total_qty,
                    top_10_asks: top_10_asks_total_qty,
                    all_bids: all_bids_total_qty,
                    all_asks: all_asks_total_qty,
                });
            }
        }
    }
}
