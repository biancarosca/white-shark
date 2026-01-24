//! Kalshi WebSocket client

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_native_tls::TlsConnector;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{error, info, warn};

use super::auth::KalshiAuth;
use super::models::*;
use crate::error::{Error, Result};

type WsStream = WebSocketStream<tokio_native_tls::TlsStream<TcpStream>>;

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
        info!("🔋 Connected to Kalshi WebSocket");

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

    pub async fn subscribe_market_lifecycle(&mut self, tickers: Option<Vec<String>>) -> Result<()> {
        self.subscribe(&[KalshiChannel::MarketLifecycle], tickers).await
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
            Some(Err(e)) => {
                warn!("WebSocket stream error: {}", e);
                Err(Error::WebSocket(e.to_string()))
            }
            None => {
                warn!("WebSocket stream returned None (connection closed)");
                Ok(None)
            }
        }
    }

    pub async fn recv(&mut self) -> Result<Option<KalshiWsMessage>> {
        match self.recv_raw().await? {
            Some(Message::Text(text)) => {
                let msg: KalshiWsMessage = serde_json::from_str(&text)?;
                Ok(Some(msg))
            }
            Some(Message::Ping(data)) => {
                if let Some(stream) = &mut self.stream {
                    let _ = stream.send(Message::Pong(data)).await;
                }
                Ok(None)
            }
            Some(Message::Close(frame)) => {
                if let Some(close_frame) = &frame {
                    warn!("WebSocket closed by server: code={:?}, reason={:?}", 
                          close_frame.code, close_frame.reason);
                } else {
                    warn!("WebSocket closed by server");
                }
                self.stream = None;
                Ok(None)
            }
            Some(Message::Binary(_)) => {
                warn!("Received unexpected binary message, ignoring");
                Ok(None)
            }
            None => {
                warn!("WebSocket stream ended unexpectedly (connection lost)");
                self.stream = None;
                Ok(None)
            }
            _ => {
                warn!("Received unexpected message type, ignoring");
                Ok(None)
            }
        }
    }

    pub async fn run(
        &mut self,
        event_tx: mpsc::Sender<KalshiEvent>,
    ) -> Result<()> {
        info!("🪁 Starting Kalshi WebSocket message loop");

        loop {
            match self.recv().await {
                Ok(Some(msg)) => {
                    if let Err(e) = self.handle_message(msg, &event_tx).await {
                        error!("Error handling message: {}", e);
                    }
                }
                Ok(None) => {
                    if self.stream.is_none() {
                        warn!("WebSocket connection lost, exiting message loop");
                        break;
                    }
                    continue;
                }
                Err(e) => {
                    error!("WebSocket receive error: {}", e);
                    self.stream = None;
                    break;
                }
            }
        }

        warn!("WebSocket message loop ended");
        Ok(())
    }

    async fn handle_message(
        &self,
        msg: KalshiWsMessage,
        event_tx: &mpsc::Sender<KalshiEvent>,
    ) -> Result<()> {
        if msg.is_subscribed() {
            info!("Subscription confirmed");
            return Ok(());
        }

        if let Some(error) = &msg.error {
            error!("WebSocket error: {}", error);
            return Ok(());
        }

        let payload = match msg.payload() {
            Some(p) => p,
            None => return Ok(()),
        };

        // Check message type first to route correctly
        match msg.msg_type.as_deref() {
            Some("orderbook_snapshot") => {
                match serde_json::from_value::<KalshiOrderbookSnapshot>(payload.clone()) {
                    Ok(snapshot) => {
                        let mut yes_bids = Vec::with_capacity(snapshot.yes_dollars.len());
                        for (p, q) in snapshot.yes_dollars {
                            if let Ok(price) = p.parse::<f64>() {
                                yes_bids.push(OrderbookLevel { price, quantity: q });
                            }
                        }
                        let mut no_bids = Vec::with_capacity(snapshot.no_dollars.len());
                        for (p, q) in snapshot.no_dollars {
                            if let Ok(price) = p.parse::<f64>() {
                                no_bids.push(OrderbookLevel { price, quantity: q });
                            }
                        }

                        let ob = KalshiOrderbook {
                            market_ticker: snapshot.market_ticker.clone(),
                            // Snapshot provides YES and NO books (resting levels). We store them as bids.
                            yes_bids,
                            yes_asks: Vec::new(),
                            no_bids,
                            no_asks: Vec::new(),
                        };

                        info!("📸 Received orderbook snapshot for {}", snapshot.market_ticker);
                        let _ = event_tx.send(KalshiEvent::OrderbookUpdate(ob)).await;
                        return Ok(());
                    }
                    Err(e) => {
                        warn!("Failed to parse orderbook snapshot: {}, payload: {:?}", e, payload);
                    }
                }
            }
            Some("orderbook_delta") => {
                match serde_json::from_value::<KalshiOrderbookDelta>(payload.clone()) {
                    Ok(delta) => {
                        info!("📊 Received orderbook delta for {}: {} {} @ {}", 
                              delta.market_ticker, delta.side, delta.delta, delta.price_dollars);
                        let _ = event_tx.send(KalshiEvent::OrderbookDelta(delta)).await;
                        return Ok(());
                    }
                    Err(e) => {
                        warn!("Failed to parse orderbook delta: {}, payload: {:?}", e, payload);
                    }
                }
            }
            _ => {}
        }

        // For other message types, check for market_ticker
        let ticker = match payload.get("market_ticker").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                // Log unhandled messages for debugging
                warn!("Unhandled message type: {:?}, payload: {:?}", msg.msg_type, payload);
                return Ok(());
            }
        };

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

        if let Ok(ticker_data) = serde_json::from_value::<KalshiTicker>(payload.clone()) {
            let _ = event_tx.send(KalshiEvent::TickerUpdate(ticker_data)).await;
        }

        Ok(())
    }
}
