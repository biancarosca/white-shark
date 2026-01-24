#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinanceStream {
    Trade,
    BestBidAsk,
    DepthPartial(u16),
}

impl BinanceStream {
    pub fn stream_name(&self, symbol: &str) -> String {
        let symbol_lower = symbol.to_lowercase();
        match self {
            BinanceStream::Trade => format!("{}@trade", symbol_lower),
            BinanceStream::BestBidAsk => format!("{}@bestBidAsk", symbol_lower),
            BinanceStream::DepthPartial(level) => format!("{}@depth{}", symbol_lower, level),
        }
    }
}

