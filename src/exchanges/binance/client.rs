use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_native_tls::TlsConnector;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};

use super::models::{BinanceMessage, BinanceStreamData, *};
use super::sbe::{build_sbe_combined_url, SbeDecoder, SbeMessage};
use crate::config::BinanceConfig;
use crate::error::{Error, Result};
use crate::exchanges::PriceUpdate;
use http::Request;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsStreamTls = WebSocketStream<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

pub enum BinanceWsStream {
    Standard(WsStream),
    Tls(WsStreamTls),
}

pub struct BinanceClient {
    config: BinanceConfig,
    stream: Option<BinanceWsStream>,
    message_id: AtomicU64,
    subscribed_streams: HashSet<String>,
    sbe_decoder: SbeDecoder,
    use_sbe: bool,
}

impl BinanceClient {
    pub fn new(config: BinanceConfig) -> Self {
        Self {
            config,
            stream: None,
            message_id: AtomicU64::new(1),
            subscribed_streams: HashSet::new(),
            sbe_decoder: SbeDecoder::new(),
            use_sbe: false,
        }
    }

    pub fn with_sbe(mut self) -> Self {
        self.use_sbe = true;
        self
    }

    fn next_id(&self) -> u64 {
        self.message_id.fetch_add(1, Ordering::SeqCst)
    }

    fn ws_url(&self, symbols: &[String]) -> String {
        if self.use_sbe {
            // Build streams list for SBE
            let streams: Vec<String> = symbols
                .iter()
                .flat_map(|s| {
                    vec![
                        BinanceStream::Trade.stream_name(s),
                        BinanceStream::BestBidAsk.stream_name(s),
                    ]
                })
                .collect();
            
            build_sbe_combined_url(&streams)
        } else {
            "wss://stream.binance.com:9443/stream".to_string()
        }
    }

    pub async fn connect(&mut self, symbols: &[String]) -> Result<()> {
        let url_str = self.ws_url(symbols);
        info!("Connecting to Binance WebSocket: {}", url_str);

        let url = url::Url::parse(&url_str)
            .map_err(|e| Error::WebSocket(format!("Invalid URL: {}", e)))?;

        // SBE requires API key and manual connection with headers
        if self.use_sbe {
            // API key is required for SBE according to Binance docs
            let api_key = self.config.api_key.as_ref()
                .ok_or_else(|| Error::WebSocket("BINANCE_API_KEY is required for SBE connections. Please set it in your environment variables.".into()))?;

            let host = url
                .host_str()
                .ok_or_else(|| Error::WebSocket("No host in URL".into()))?;
            let port = url.port_or_known_default().unwrap_or(443);

            let ws_key = generate_key();
            let request = Request::builder()
                .uri(&url_str)
                .header("Host", host)
                .header("Upgrade", "websocket")
                .header("Connection", "Upgrade")
                .header("Sec-WebSocket-Key", &ws_key)
                .header("Sec-WebSocket-Version", "13")
                .header("X-MBX-APIKEY", api_key)
                .body(())
                .map_err(|e| Error::WebSocket(format!("Failed to build request: {}", e)))?;
            
            // Log request details for debugging
            debug!("WebSocket request URI: {}", url_str);
            debug!("WebSocket request headers: Host={}, X-MBX-APIKEY={}***", host, &api_key[..api_key.len().min(8)]);

            let tcp_stream = TcpStream::connect(format!("{}:{}", host, port))
                .await
                .map_err(|e| Error::WebSocket(format!("TCP connection failed: {}", e)))?;

            let tls_connector = native_tls::TlsConnector::builder()
                .build()
                .map_err(|e| Error::WebSocket(format!("TLS connector failed: {}", e)))?;
            let tls_connector = TlsConnector::from(tls_connector);
            let tls_stream = tls_connector
                .connect(host, tcp_stream)
                .await
                .map_err(|e| Error::WebSocket(format!("TLS connection failed: {}", e)))?;

            let (stream, response) = client_async(request, tls_stream)
                .await
                .map_err(|e| {
                    // Extract detailed error information
                    let error_msg = match &e {
                        tokio_tungstenite::tungstenite::Error::Http(response) => {
                            let status = response.status();
                            let status_text = response.status().canonical_reason().unwrap_or("Unknown");
                            let mut msg = format!("HTTP {} {}", status.as_u16(), status_text);
                            
                            // Try to get response body if available
                            if let Some(body) = response.body() {
                                if let Ok(body_str) = std::str::from_utf8(body) {
                                    msg.push_str(&format!(" - Response body: {}", body_str));
                                } else {
                                    msg.push_str(&format!(" - Response body (hex): {:?}", body));
                                }
                            }
                            
                            // Include headers that might be useful
                            let headers: Vec<String> = response
                                .headers()
                                .iter()
                                .map(|(k, v)| format!("{}: {:?}", k, v))
                                .collect();
                            if !headers.is_empty() {
                                msg.push_str(&format!(" - Headers: {}", headers.join(", ")));
                            }
                            
                            msg
                        }
                        _ => e.to_string(),
                    };
                    Error::WebSocket(format!("Connection failed: {}", error_msg))
                })?;
            
            // Log the response for debugging
            debug!("WebSocket handshake response: {:?}", response);

            self.stream = Some(BinanceWsStream::Tls(stream));
        } else {
            // For non-SBE, use simple connection
            let (stream, _) = connect_async(&url_str)
                .await
                .map_err(|e| Error::WebSocket(e.to_string()))?;

            self.stream = Some(BinanceWsStream::Standard(stream));
        }

        info!("Connected to Binance WebSocket");
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(stream) = self.stream.take() {
            match stream {
                BinanceWsStream::Standard(mut s) => {
                    s.close(None).await.map_err(|e| Error::WebSocket(e.to_string()))?;
                }
                BinanceWsStream::Tls(mut s) => {
                    s.close(None).await.map_err(|e| Error::WebSocket(e.to_string()))?;
                }
            }
        }
        self.subscribed_streams.clear();
        info!("Disconnected from Binance WebSocket");
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    async fn send_json<T: serde::Serialize>(&mut self, msg: &T) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        debug!("Sending: {}", json);

        match &mut self.stream {
            Some(BinanceWsStream::Standard(s)) => {
                s.send(Message::Text(json))
                    .await
                    .map_err(|e| Error::WebSocket(e.to_string()))?;
            }
            Some(BinanceWsStream::Tls(s)) => {
                s.send(Message::Text(json))
                    .await
                    .map_err(|e| Error::WebSocket(e.to_string()))?;
            }
            None => return Err(Error::WebSocket("Not connected".into())),
        }

        Ok(())
    }

    pub async fn subscribe(&mut self, streams: Vec<String>) -> Result<()> {
        let msg = SubscribeRequest::subscribe(self.next_id(), streams.clone());
        self.send_json(&msg).await?;

        for stream in streams {
            self.subscribed_streams.insert(stream);
        }

        Ok(())
    }

    pub async fn unsubscribe(&mut self, streams: Vec<String>) -> Result<()> {
        let msg = SubscribeRequest::unsubscribe(self.next_id(), streams.clone());
        self.send_json(&msg).await?;

        for stream in streams {
            self.subscribed_streams.remove(&stream);
        }

        Ok(())
    }

    pub async fn subscribe_trades(&mut self, symbol: &str) -> Result<()> {
        let stream = BinanceStream::Trade.stream_name(symbol);
        self.subscribe(vec![stream]).await
    }

    pub async fn subscribe_best_bid_ask(&mut self, symbol: &str) -> Result<()> {
        let stream = BinanceStream::BestBidAsk.stream_name(symbol);
        self.subscribe(vec![stream]).await
    }

    pub async fn subscribe_depth(&mut self, symbol: &str) -> Result<()> {
        let stream = BinanceStream::Depth20.stream_name(symbol);
        self.subscribe(vec![stream]).await
    }

    pub async fn subscribe_kline(&mut self, symbol: &str, interval: KlineInterval) -> Result<()> {
        let stream = BinanceStream::Kline(interval).stream_name(symbol);
        self.subscribe(vec![stream]).await
    }

    pub async fn subscribe_prices(&mut self, symbols: &[String]) -> Result<()> {
        let streams: Vec<String> = symbols
            .iter()
            .flat_map(|s| {
                vec![
                    BinanceStream::Trade.stream_name(s),
                    BinanceStream::BestBidAsk.stream_name(s),
                ]
            })
            .collect();

        self.subscribe(streams).await
    }

    pub async fn recv_raw(&mut self) -> Result<Option<Message>> {
        match &mut self.stream {
            Some(BinanceWsStream::Standard(s)) => {
                match s.next().await {
                    Some(Ok(msg)) => Ok(Some(msg)),
                    Some(Err(e)) => Err(Error::WebSocket(e.to_string())),
                    None => Ok(None),
                }
            }
            Some(BinanceWsStream::Tls(s)) => {
                match s.next().await {
                    Some(Ok(msg)) => Ok(Some(msg)),
                    Some(Err(e)) => Err(Error::WebSocket(e.to_string())),
                    None => Ok(None),
                }
            }
            None => Err(Error::WebSocket("Not connected".into())),
        }
    }

    pub async fn recv_json(&mut self) -> Result<Option<BinanceMessage>> {
        match self.recv_raw().await? {
            Some(Message::Text(text)) => {
                debug!("Received: {}", text);
                match BinanceMessage::from_json(&text) {
                    Ok(msg) => Ok(Some(msg)),
                    Err(e) => {
                        warn!("Failed to parse message: {} - {}", e, text);
                        Ok(None)
                    }
                }
            }
            Some(Message::Ping(data)) => {
                debug!("Received ping, sending pong");
                match &mut self.stream {
                    Some(BinanceWsStream::Standard(s)) => {
                        if let Err(e) = s.send(Message::Pong(data)).await {
                            warn!("Failed to send pong: {}", e);
                            return Err(Error::WebSocket(format!("Failed to send pong: {}", e)));
                        }
                    }
                    Some(BinanceWsStream::Tls(s)) => {
                        if let Err(e) = s.send(Message::Pong(data)).await {
                            warn!("Failed to send pong: {}", e);
                            return Err(Error::WebSocket(format!("Failed to send pong: {}", e)));
                        }
                    }
                    None => {
                        return Err(Error::WebSocket("Stream is None when trying to send pong".into()));
                    }
                }
                Ok(None) // Continue loop after handling ping
            }
            Some(Message::Pong(_)) => {
                debug!("Received unsolicited pong");
                Ok(None) // Continue loop
            }
            Some(Message::Close(frame)) => {
                if let Some(close_frame) = &frame {
                    info!("WebSocket closed by server: code={:?}, reason={:?}", close_frame.code, close_frame.reason);
                } else {
                    info!("WebSocket closed by server");
                }
                self.stream = None;
                Err(Error::WebSocket("WebSocket connection closed".into()))
            }
            Some(Message::Binary(_)) => {
                warn!("Received unexpected binary message in JSON mode");
                Ok(None) // Continue loop
            }
            Some(Message::Frame(_)) => {
                debug!("Received raw frame (unexpected)");
                Ok(None) // Continue loop
            }
            None => {
                warn!("WebSocket stream ended (received None)");
                self.stream = None;
                Err(Error::WebSocket("WebSocket stream ended".into()))
            }
        }
    }

    pub async fn recv_sbe(&mut self) -> Result<Option<SbeMessage>> {
        match self.recv_raw().await? {
            Some(Message::Binary(data)) => {
                let msg = self.sbe_decoder.decode(&data)?;
                Ok(Some(msg))
            }
            Some(Message::Ping(data)) => {
                debug!("Received ping, sending pong");
                match &mut self.stream {
                    Some(BinanceWsStream::Standard(s)) => {
                        if let Err(e) = s.send(Message::Pong(data)).await {
                            warn!("Failed to send pong: {}", e);
                            return Err(Error::WebSocket(format!("Failed to send pong: {}", e)));
                        }
                    }
                    Some(BinanceWsStream::Tls(s)) => {
                        if let Err(e) = s.send(Message::Pong(data)).await {
                            warn!("Failed to send pong: {}", e);
                            return Err(Error::WebSocket(format!("Failed to send pong: {}", e)));
                        }
                    }
                    None => {
                        return Err(Error::WebSocket("Stream is None when trying to send pong".into()));
                    }
                }
                Ok(None) // Continue loop after handling ping
            }
            Some(Message::Pong(_)) => {
                debug!("Received unsolicited pong");
                Ok(None) // Continue loop
            }
            Some(Message::Close(frame)) => {
                if let Some(close_frame) = &frame {
                    info!("WebSocket closed by server: code={:?}, reason={:?}", close_frame.code, close_frame.reason);
                } else {
                    info!("WebSocket closed by server");
                }
                self.stream = None;
                Err(Error::WebSocket("WebSocket connection closed".into()))
            }
            Some(Message::Text(text)) => {
                warn!("Received unexpected text message in SBE mode: {}", text);
                Ok(None) // Continue loop
            }
            Some(Message::Frame(_)) => {
                debug!("Received raw frame (unexpected)");
                Ok(None) // Continue loop
            }
            None => {
                warn!("WebSocket stream ended (received None)");
                self.stream = None;
                Err(Error::WebSocket("WebSocket stream ended".into()))
            }
        }
    }

    pub async fn run(&mut self, price_tx: mpsc::Sender<PriceUpdate>) -> Result<()> {
        info!("Starting Binance message loop");

        if self.use_sbe {
            loop {
                let received_at = chrono::Utc::now();
                match self.recv_sbe().await {
                    Ok(Some(msg)) => {
                        info!("[SBE] Message received at: {}", received_at.format("%Y-%m-%dT%H:%M:%S%.6fZ"));
                        let update = msg.to_price_update();
                        if price_tx.send(update).await.is_err() {
                            warn!("Price channel closed");
                            break;
                        }
                    }
                    Ok(None) => {
                        // Ping/pong handled, continue loop
                        continue;
                    }
                    Err(e) => {
                        error!("Error receiving SBE message: {}", e);
                        return Err(e);
                    }
                }
            }
        } else {
            loop {
                let received_at = chrono::Utc::now();
                match self.recv_json().await {
                    Ok(Some(msg)) => {
                        match msg {
                            BinanceMessage::SubscriptionResponse(response) => {
                                if let Some(error) = response.error {
                                    error!("Binance subscription error: {:?}", error);
                                } else {
                                    debug!("Subscription confirmed (id: {})", response.id);
                                }
                            }
                            BinanceMessage::StreamMessage(stream_msg) => {
                                info!("[JSON] Message received at: {}", received_at.format("%Y-%m-%dT%H:%M:%S%.6fZ"));
                                match stream_msg.parse_data() {
                                    BinanceStreamData::Trade(trade) => {
                                        let update = trade.to_price_update();
                                        if price_tx.send(update).await.is_err() {
                                            warn!("Price channel closed");
                                            break;
                                        }
                                    }
                                    BinanceStreamData::BestBidAsk(bba) => {
                                        let update = bba.to_price_update();
                                        if price_tx.send(update).await.is_err() {
                                            warn!("Price channel closed");
                                            break;
                                        }
                                    }
                                    BinanceStreamData::Kline(kline) => {
                                        if let Some((_, _, _, close, _)) = kline.kline.ohlcv() {
                                            let update = PriceUpdate {
                                                exchange: "binance".to_string(),
                                                symbol: kline.symbol,
                                                timestamp: chrono::Utc::now(),
                                                bid: None,
                                                ask: None,
                                                last_price: Some(close),
                                                volume_24h: None,
                                            };
                                            if price_tx.send(update).await.is_err() {
                                                warn!("Price channel closed");
                                                break;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // Ping/pong handled, continue loop
                        continue;
                    }
                    Err(e) => {
                        error!("Error receiving JSON message: {}", e);
                        return Err(e);
                    }
                }
            }
        }

        warn!("WebSocket message loop ended");
        Ok(())
    }

    pub async fn start(&mut self, symbols: &[String], price_tx: mpsc::Sender<PriceUpdate>) -> Result<()> {
        if !self.is_connected() {
            self.connect(symbols).await?;
        }

        // For SBE, streams are already in the URL, so no need to subscribe
        if !self.use_sbe {
            self.subscribe_prices(symbols).await?;
        }
        
        self.run(price_tx).await?;

        Ok(())
    }
}

pub struct BinanceClientBuilder {
    config: Option<BinanceConfig>,
    use_sbe: bool,
}

impl BinanceClientBuilder {
    pub fn new() -> Self {
        Self {
            config: None,
            use_sbe: false,
        }
    }

    pub fn config(mut self, config: BinanceConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn use_sbe(mut self, enable: bool) -> Self {
        self.use_sbe = enable;
        self
    }

    pub fn symbols(mut self, symbols: Vec<String>) -> Self {
        if let Some(ref mut config) = self.config {
            config.tracked_symbols = symbols;
        } else {
            self.config = Some(BinanceConfig {
                tracked_symbols: symbols,
                ..Default::default()
            });
        }
        self
    }

    pub fn build(self) -> BinanceClient {
        let config = self.config.unwrap_or_default();
        let mut client = BinanceClient::new(config);
        if self.use_sbe {
            client = client.with_sbe();
        }
        client
    }
}

impl Default for BinanceClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
