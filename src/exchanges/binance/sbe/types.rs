use zerocopy::{FromBytes, FromZeroes, Ref, Unaligned};
use zerocopy::byteorder::{LittleEndian, U16};
use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

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

#[repr(C)]
#[derive(FromZeroes, FromBytes, Unaligned)]
struct MessageHeaderRaw {
    block_length: U16<LittleEndian>,
    template_id: U16<LittleEndian>,
    schema_id: U16<LittleEndian>,
    version: U16<LittleEndian>,
}

impl MessageHeader {
    pub const SIZE: usize = 8;

    pub fn decode(data: &[u8]) -> Result<Self> {
        let (raw, _) = Ref::<_, MessageHeaderRaw>::new_from_prefix(data)
            .ok_or_else(|| Error::SbeDecode("Header too short".into()))?;
        Ok(Self {
            block_length: raw.block_length.get(),
            template_id: raw.template_id.get(),
            schema_id: raw.schema_id.get(),
            version: raw.version.get(),
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

