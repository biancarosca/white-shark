use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_native_tls::TlsConnector;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};
use tracing::{debug, error, info, warn};

use super::sbe::{decoder::SbeDecoder, messages::SbeMessage, url::build_sbe_combined_url};
use crate::config::BinanceConfig;
use crate::error::{Error, Result};
use crate::exchanges::PriceUpdate;
use http::Request;

type WsStream = WebSocketStream<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

pub struct BinanceClient {
    config: BinanceConfig,
    stream: Option<WsStream>,
    sbe_decoder: SbeDecoder,
    recv_buf: Vec<u8>,
}

impl BinanceClient {
    pub fn new(config: BinanceConfig) -> Self {
        Self {
            config,
            stream: None,
            sbe_decoder: SbeDecoder::new(),
            recv_buf: Vec::new(),
        }
    }

    fn ws_url(&self, symbols: &[String]) -> String {
        let mut streams = Vec::with_capacity(symbols.len() * 3);
        for symbol in symbols {
            let symbol_lower = symbol.to_ascii_lowercase();
            streams.push(format!("{}@trade", symbol_lower));
            streams.push(format!("{}@bestBidAsk", symbol_lower));
            streams.push(format!("{}@depth{}", symbol_lower, 20));
        }

        build_sbe_combined_url(&streams)
    }

    pub async fn connect(&mut self, symbols: &[String]) -> Result<()> {
        let url_str = self.ws_url(symbols);
        info!("Connecting to Binance WebSocket: {}", url_str);

        let url = url::Url::parse(&url_str)
            .map_err(|e| Error::WebSocket(format!("Invalid URL: {}", e)))?;

        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| {
                Error::WebSocket(
                    "BINANCE_API_KEY is required for SBE connections. \
                    Please set it in your environment variables."
                        .into(),
                )
            })?;

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

        let (stream, _) = client_async(request, tls_stream).await.map_err(|e| {
            let error_msg = match &e {
                tokio_tungstenite::tungstenite::Error::Http(response) => {
                    let status = response.status();
                    let status_text = response
                        .status()
                        .canonical_reason()
                        .unwrap_or("Unknown");
                    let mut msg = format!("HTTP {} {}", status.as_u16(), status_text);

                    if let Some(body) = response.body() {
                        if let Ok(body_str) = std::str::from_utf8(body) {
                            msg.push_str(&format!(" - Response body: {}", body_str));
                        } else {
                            msg.push_str(&format!(" - Response body (hex): {:?}", body));
                        }
                    }

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

        self.stream = Some(stream);

        info!("Connected to Binance WebSocket");
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(stream) = self.stream.take() {
            let mut stream = stream;
            stream
                .close(None)
                .await
                .map_err(|e| Error::WebSocket(e.to_string()))?;
        }
        info!("Disconnected from Binance WebSocket");
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    pub async fn recv_raw(&mut self) -> Result<Option<Message>> {
        match &mut self.stream {
            Some(s) => match s.next().await {
                Some(Ok(msg)) => Ok(Some(msg)),
                Some(Err(e)) => Err(Error::WebSocket(e.to_string())),
                None => Ok(None),
            },
            None => Err(Error::WebSocket("Not connected".into())),
        }
    }

    pub async fn recv_sbe<'a>(&'a mut self) -> Result<Option<SbeMessage<'a>>> {
        match self.recv_raw().await? {
            Some(Message::Binary(data)) => {
                self.recv_buf = data;
                let msg = self.sbe_decoder.decode(&self.recv_buf)?;
                Ok(Some(msg))
            }
            Some(Message::Ping(data)) => {
                debug!("Received ping, sending pong");
                match &mut self.stream {
                    Some(s) => {
                        if let Err(e) = s.send(Message::Pong(data)).await {
                            warn!("Failed to send pong: {}", e);
                            return Err(Error::WebSocket(format!("Failed to send pong: {}", e)));
                        }
                    }
                    None => {
                        return Err(Error::WebSocket("Stream is None when trying to send pong".into()));
                    }
                }
                Ok(None)
            }
            Some(Message::Pong(_)) => {
                warn!("Received unsolicited pong");
                Ok(None)
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
                Ok(None)
            }
            Some(Message::Frame(_)) => {
                debug!("Received raw frame (unexpected)");
                Ok(None)
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

        let _ = price_tx;
        loop {
            match self.recv_sbe().await {
                Ok(Some(msg)) => {
                    msg.print_update();
                }
                Ok(None) => {
                    continue;
                }
                Err(e) => {
                    error!("Error receiving SBE message: {}", e);
                    return Err(e);
                }
            }
        }
    }

    pub async fn start(&mut self, symbols: &[String], price_tx: mpsc::Sender<PriceUpdate>) -> Result<()> {
        if !self.is_connected() {
            self.connect(symbols).await?;
        }
        
        self.run(price_tx).await?;

        Ok(())
    }
}
