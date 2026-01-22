//! Kalshi WebSocket client

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_native_tls::TlsConnector;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, error, info, warn};

use super::auth::KalshiAuth;
use super::models::*;
use crate::error::{Error, Result};

type WsStream = WebSocketStream<tokio_native_tls::TlsStream<TcpStream>>;

/// Kalshi WebSocket client
pub struct KalshiWebSocket {
    url: String,
    auth: KalshiAuth,
    stream: Option<WsStream>,
    message_id: AtomicU64,
    subscribed_markets: HashSet<String>,
    subscribed_channels: HashSet<String>,
}

impl KalshiWebSocket {
    pub fn new(url: &str, auth: KalshiAuth) -> Self {
        Self {
            url: url.to_string(),
            auth,
            stream: None,
            message_id: AtomicU64::new(1),
            subscribed_markets: HashSet::new(),
            subscribed_channels: HashSet::new(),
        }
    }

    fn next_id(&self) -> u64 {
        self.message_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Kalshi WebSocket: {}", self.url);

        let url = url::Url::parse(&self.url)?;
        let host = url
            .host_str()
            .ok_or_else(|| Error::Connection("No host in URL".into()))?;
        let port = url.port_or_known_default().unwrap_or(443);

        let auth_headers = self.auth.generate_ws_headers()?;

        let request = http::Request::builder()
            .uri(&self.url)
            .header("Host", host)
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Key", generate_key())
            .header("Sec-WebSocket-Version", "13")
            .header("KALSHI-ACCESS-KEY", &auth_headers.api_key)
            .header("KALSHI-ACCESS-TIMESTAMP", &auth_headers.timestamp)
            .header("KALSHI-ACCESS-SIGNATURE", &auth_headers.signature)
            .body(())
            .map_err(|e| Error::Connection(e.to_string()))?;

        let tcp_stream = TcpStream::connect(format!("{}:{}", host, port))
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;

        let tls_connector = native_tls::TlsConnector::builder()
            .build()
            .map_err(|e| Error::Tls(e.to_string()))?;
        let tls_connector = TlsConnector::from(tls_connector);
        let tls_stream = tls_connector
            .connect(host, tcp_stream)
            .await
            .map_err(|e| Error::Tls(e.to_string()))?;

        let (ws_stream, _) = tokio_tungstenite::client_async(request, tls_stream)
            .await
            .map_err(|e| Error::WebSocket(e.to_string()))?;

        self.stream = Some(ws_stream);
        info!("Connected to Kalshi WebSocket");

        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            stream
                .close(None)
                .await
                .map_err(|e| Error::WebSocket(e.to_string()))?;
        }
        self.subscribed_markets.clear();
        self.subscribed_channels.clear();
        info!("Disconnected from Kalshi WebSocket");
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    async fn send_message<T: serde::Serialize>(&mut self, msg: &T) -> Result<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))?;

        let json = serde_json::to_string(msg)?;
        debug!("Sending: {}", json);

        stream
            .send(Message::Text(json))
            .await
            .map_err(|e| Error::WebSocket(e.to_string()))?;

        Ok(())
    }

    pub async fn subscribe(
        &mut self,
        channels: &[KalshiChannel],
        tickers: Option<Vec<String>>,
    ) -> Result<()> {
        let channel_strs: Vec<String> = channels.iter().map(|c| c.as_str().to_string()).collect();
        let msg = SubscribeMessage::new(self.next_id(), channel_strs.clone(), tickers.clone());
        self.send_message(&msg).await?;

        for channel in channel_strs {
            self.subscribed_channels.insert(channel);
        }
        if let Some(t) = tickers {
            for ticker in t {
                self.subscribed_markets.insert(ticker);
            }
        }

        Ok(())
    }

    pub async fn unsubscribe(
        &mut self,
        channels: &[KalshiChannel],
        tickers: Option<Vec<String>>,
    ) -> Result<()> {
        let channel_strs: Vec<String> = channels.iter().map(|c| c.as_str().to_string()).collect();
        let msg = UnsubscribeMessage::new(self.next_id(), channel_strs.clone(), tickers.clone());
        self.send_message(&msg).await?;

        if let Some(t) = tickers {
            for ticker in t {
                self.subscribed_markets.remove(&ticker);
            }
        }

        Ok(())
    }

    pub async fn subscribe_market_lifecycle(&mut self) -> Result<()> {
        self.subscribe(&[KalshiChannel::MarketLifecycle], None).await
    }

    pub async fn subscribe_tickers(&mut self, tickers: Vec<String>) -> Result<()> {
        self.subscribe(&[KalshiChannel::Ticker], Some(tickers)).await
    }

    pub async fn subscribe_all_tickers(&mut self) -> Result<()> {
        self.subscribe(&[KalshiChannel::Ticker], None).await
    }

    pub async fn subscribe_orderbook(&mut self, tickers: Vec<String>) -> Result<()> {
        self.subscribe(&[KalshiChannel::OrderbookDelta], Some(tickers))
            .await
    }

    pub async fn recv_raw(&mut self) -> Result<Option<Message>> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::WebSocket("Not connected".into()))?;

        match stream.next().await {
            Some(Ok(msg)) => Ok(Some(msg)),
            Some(Err(e)) => Err(Error::WebSocket(e.to_string())),
            None => Ok(None),
        }
    }

    pub async fn recv(&mut self) -> Result<Option<KalshiWsMessage>> {
        match self.recv_raw().await? {
            Some(Message::Text(text)) => {
                debug!("Received: {}", text);
                let msg: KalshiWsMessage = serde_json::from_str(&text)?;
                Ok(Some(msg))
            }
            Some(Message::Ping(data)) => {
                if let Some(stream) = &mut self.stream {
                    let _ = stream.send(Message::Pong(data)).await;
                }
                Ok(None)
            }
            Some(Message::Close(_)) => {
                info!("WebSocket closed by server");
                self.stream = None;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Run message loop, sending events to channel
    pub async fn run(
        &mut self,
        event_tx: mpsc::Sender<KalshiEvent>,
        market_filter: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    ) -> Result<()> {
        info!("Starting Kalshi WebSocket message loop");

        while let Some(msg) = self.recv().await? {
            if let Err(e) = self.handle_message(msg, &event_tx, &market_filter).await {
                error!("Error handling message: {}", e);
            }
        }

        warn!("WebSocket message loop ended");
        Ok(())
    }

    async fn handle_message(
        &self,
        msg: KalshiWsMessage,
        event_tx: &mpsc::Sender<KalshiEvent>,
        market_filter: &Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    ) -> Result<()> {
        if msg.is_subscribed() {
            debug!("Subscription confirmed");
            return Ok(());
        }

        if let Some(error) = &msg.error {
            error!("WebSocket error: {}", error);
            return Ok(());
        }

        if let Some(payload) = msg.payload() {
            if let Some(ticker) = payload.get("market_ticker").and_then(|v| v.as_str()) {
                // Apply filter if provided
                if let Some(filter) = market_filter {
                    if !filter(ticker) {
                        return Ok(());
                    }
                }

                // Check if it's a lifecycle event
                if let Some(new_status) = payload.get("new_status").and_then(|v| v.as_str()) {
                    let old_status = payload
                        .get("old_status")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let event = KalshiEvent::MarketStatusChanged {
                        ticker: ticker.to_string(),
                        old_status,
                        new_status: new_status.to_string(),
                    };

                    let _ = event_tx.send(event).await;
                    return Ok(());
                }

                // Otherwise treat as ticker update
                if let Ok(ticker_data) = serde_json::from_value::<KalshiTicker>(payload.clone()) {
                    let _ = event_tx.send(KalshiEvent::TickerUpdate(ticker_data)).await;
                }
            }
        }

        Ok(())
    }
}

/// Builder for KalshiWebSocket
pub struct KalshiWebSocketBuilder {
    url: Option<String>,
    api_key: Option<String>,
    private_key_path: Option<String>,
}

impl KalshiWebSocketBuilder {
    pub fn new() -> Self {
        Self {
            url: None,
            api_key: None,
            private_key_path: None,
        }
    }

    pub fn url(mut self, url: &str) -> Self {
        self.url = Some(url.to_string());
        self
    }

    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = Some(key.to_string());
        self
    }

    pub fn private_key_path(mut self, path: &str) -> Self {
        self.private_key_path = Some(path.to_string());
        self
    }

    pub fn build(self) -> Result<KalshiWebSocket> {
        let url = self
            .url
            .unwrap_or_else(|| "wss://api.elections.kalshi.com/trade-api/ws/v2".to_string());

        let api_key = self
            .api_key
            .ok_or_else(|| Error::Config("API key required".into()))?;

        let private_key_path = self
            .private_key_path
            .ok_or_else(|| Error::Config("Private key path required".into()))?;

        let auth = KalshiAuth::from_file(&api_key, &private_key_path)?;

        Ok(KalshiWebSocket::new(&url, auth))
    }
}

impl Default for KalshiWebSocketBuilder {
    fn default() -> Self {
        Self::new()
    }
}
