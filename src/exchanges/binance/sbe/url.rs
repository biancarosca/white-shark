use crate::constants::BINANCE_SBE_WS_URL;

pub fn build_sbe_stream_url(symbol: &str, stream_type: &str) -> String {
    format!(
        "{}/ws/{}@{}",
        BINANCE_SBE_WS_URL,
        symbol.to_lowercase(),
        stream_type
    )
}

pub fn build_sbe_combined_url(streams: &[String]) -> String {
    let streams_param = streams.join("/");
    format!(
        "{}/stream?streams={}",
        BINANCE_SBE_WS_URL,
        streams_param
    )
} 

