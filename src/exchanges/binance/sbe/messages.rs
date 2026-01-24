use chrono::{DateTime, Utc};

use crate::exchanges::binance::sbe::events::{
    bid_ask::BestBidAskStreamEvent,
    depth::DepthSnapshotStreamEvent,
    trade::TradeStreamEvent,
};


#[derive(Debug, Clone)]
pub enum SbeMessage<'a> {
    Trade(TradeStreamEvent<'a>),
    BestBidAsk(BestBidAskStreamEvent<'a>),
    DepthSnapshot(DepthSnapshotStreamEvent<'a>),
}

impl<'a> SbeMessage<'a> {
    pub fn print_update(&self) {
        match self {
            SbeMessage::Trade(e) => e.print_update(),
            SbeMessage::BestBidAsk(e) => e.print_update(),
            SbeMessage::DepthSnapshot(e) => e.print_update(),
        }
    }

    pub fn symbol(&self) -> &'a str {
        match self {
            SbeMessage::Trade(e) => &e.symbol,
            SbeMessage::BestBidAsk(e) => &e.symbol,
            SbeMessage::DepthSnapshot(e) => &e.symbol,
        }
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            SbeMessage::Trade(e) => e.event_time,
            SbeMessage::BestBidAsk(e) => e.event_time,
            SbeMessage::DepthSnapshot(e) => e.event_time,
        }
    }
}
