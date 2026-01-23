use tracing::warn;
use tracing::error;

use crate::error::{Error, Result};

use super::messages::*;
use super::types::*;

pub struct SbeDecoder {
    pub expected_schema_id: u16,
    pub expected_version: u16,
}

impl SbeDecoder {
    pub fn new() -> Self {
        Self {
            expected_schema_id: SCHEMA_ID,
            expected_version: SCHEMA_VERSION,
        }
    }

    pub fn with_schema(schema_id: u16, version: u16) -> Self {
        Self {
            expected_schema_id: schema_id,
            expected_version: version,
        }
    }

    pub fn decode(&self, data: &[u8]) -> Result<SbeMessage> {
        let header = MessageHeader::decode(data)?;

        if header.schema_id != self.expected_schema_id {
            warn!(
                "Schema ID from server: {} (expected {}), Version: {}",
                header.schema_id,
                self.expected_schema_id,
                header.version
            );
        }

        if header.version != self.expected_version {
            warn!(
                "Version from server: {} (expected {}), Schema ID: {}",
                header.version,
                self.expected_version,
                header.schema_id
            );
        }

        let body = &data[MessageHeader::SIZE..];

        match header.message_type() {
            SbeMessageType::Trade => {
                TradeStreamEvent::decode(body)
                    .map(SbeMessage::Trade)
                    .map_err(|e| {
                        error!("Failed to decode Trade message (body_len={}): {}", body.len(), e);
                        e
                    })
            }
            SbeMessageType::BestBidAsk => {
                BestBidAskStreamEvent::decode(body)
                    .map(SbeMessage::BestBidAsk)
                    .map_err(|e| {
                        error!("Failed to decode BestBidAsk message (body_len={}): {}", body.len(), e);
                        e
                    })
            }
            SbeMessageType::DepthDiff => {
                DepthDiffStreamEvent::decode(body)
                    .map(SbeMessage::DepthDiff)
                    .map_err(|e| {
                        tracing::error!("Failed to decode DepthDiff message (body_len={}): {}", body.len(), e);
                        e
                    })
            }
            SbeMessageType::DepthSnapshot => {
                DepthSnapshotStreamEvent::decode(body)
                    .map(SbeMessage::DepthSnapshot)
                    .map_err(|e| {
                        tracing::error!("Failed to decode DepthSnapshot message (body_len={}): {}", body.len(), e);
                        e
                    })
            }
            SbeMessageType::Unknown(id) => {
                error!(
                    "Unknown template ID: {} (schema_id={}, version={}, block_length={}, body_len={})",
                    id,
                    header.schema_id,
                    header.version,
                    header.block_length,
                    body.len()
                );
                Err(Error::SbeDecode(format!("Unknown template ID: {}", id)))
            }
        }
    }
}

impl Default for SbeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

