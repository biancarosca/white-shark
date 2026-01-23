use std::io::Cursor;

use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

// From stream_1_0.xml schema
pub const SCHEMA_ID: u16 = 1;
pub const SCHEMA_VERSION: u16 = 0;

pub const TEMPLATE_TRADES_STREAM: u16 = 10000;
pub const TEMPLATE_BEST_BID_ASK_STREAM: u16 = 10001;
pub const TEMPLATE_DEPTH_SNAPSHOT_STREAM: u16 = 10002;
pub const TEMPLATE_DEPTH_DIFF_STREAM: u16 = 10003;

#[derive(Debug, Clone, Copy)]
pub struct MessageHeader {
    pub block_length: u16,
    pub template_id: u16,
    pub schema_id: u16,
    pub version: u16,
}

impl MessageHeader {
    pub const SIZE: usize = 8;

    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            return Err(Error::SbeDecode("Header too short".into()));
        }

        let mut cursor = Cursor::new(data);
        Ok(Self {
            block_length: cursor.read_u16::<LittleEndian>()?,
            template_id: cursor.read_u16::<LittleEndian>()?,
            schema_id: cursor.read_u16::<LittleEndian>()?,
            version: cursor.read_u16::<LittleEndian>()?,
        })
    }

    pub fn message_type(&self) -> SbeMessageType {
        SbeMessageType::from_template_id(self.template_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbeMessageType {
    Trade,
    BestBidAsk,
    DepthDiff,
    DepthSnapshot,
    Unknown(u16),
}

impl SbeMessageType {
    pub fn from_template_id(id: u16) -> Self {
        match id {
            TEMPLATE_TRADES_STREAM => SbeMessageType::Trade,
            TEMPLATE_BEST_BID_ASK_STREAM => SbeMessageType::BestBidAsk,
            TEMPLATE_DEPTH_DIFF_STREAM => SbeMessageType::DepthDiff,
            TEMPLATE_DEPTH_SNAPSHOT_STREAM => SbeMessageType::DepthSnapshot,
            _ => SbeMessageType::Unknown(id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggressorSide {
    Buy,
    Sell,
    Unknown(u8),
}

impl From<u8> for AggressorSide {
    fn from(v: u8) -> Self {
        match v {
            0 => AggressorSide::Buy,
            1 => AggressorSide::Sell,
            _ => AggressorSide::Unknown(v),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthUpdateAction {
    New,
    Change,
    Delete,
    Unknown(u8),
}

impl From<u8> for DepthUpdateAction {
    fn from(v: u8) -> Self {
        match v {
            0 => DepthUpdateAction::New,
            1 => DepthUpdateAction::Change,
            2 => DepthUpdateAction::Delete,
            _ => DepthUpdateAction::Unknown(v),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthSide {
    Bid,
    Ask,
    Unknown(u8),
}

impl From<u8> for DepthSide {
    fn from(v: u8) -> Self {
        match v {
            0 => DepthSide::Bid,
            1 => DepthSide::Ask,
            _ => DepthSide::Unknown(v),
        }
    }
}

#[inline]
pub fn decode_decimal(mantissa: i64, exponent: i8) -> f64 {
    mantissa as f64 * 10f64.powi(exponent as i32)
}

#[inline]
pub fn micros_to_datetime(micros: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(micros as i64).unwrap_or_else(Utc::now)
}

pub fn read_symbol(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

