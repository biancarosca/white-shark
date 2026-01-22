//! Common WebSocket utilities

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
    MaybeTlsStream, WebSocketStream,
};

use crate::error::{Error, Result};

/// WebSocket connection wrapper with common functionality
pub struct WsConnection {
    pub stream: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    pub url: String,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}

impl WsConnection {
    pub fn new(url: &str) -> Self {
        Self {
            stream: None,
            url: url.to_string(),
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(60),
        }
    }

    pub fn with_timeouts(mut self, connect: Duration, read: Duration) -> Self {
        self.connect_timeout = connect;
        self.read_timeout = read;
        self
    }

    /// Connect to WebSocket (handles TLS automatically via tokio-tungstenite)
    pub async fn connect(&mut self) -> Result<()> {
        let (stream, _) = timeout(
            self.connect_timeout,
            connect_async(&self.url),
        )
        .await
        .map_err(|_| Error::Connection("Connection timeout".into()))?
        .map_err(|e| Error::WebSocket(e.to_string()))?;

        self.stream = Some(stream);
        Ok(())
    }

    /// Send a text message
    pub async fn send(&mut self, msg: &str) -> Result<()> {
        if let Some(stream) = &mut self.stream {
            stream
                .send(Message::Text(msg.to_string()))
                .await
                .map_err(|e| Error::WebSocket(e.to_string()))?;
            Ok(())
        } else {
            Err(Error::WebSocket("Not connected".into()))
        }
    }

    /// Send a binary message
    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        if let Some(stream) = &mut self.stream {
            stream
                .send(Message::Binary(data))
                .await
                .map_err(|e| Error::WebSocket(e.to_string()))?;
            Ok(())
        } else {
            Err(Error::WebSocket("Not connected".into()))
        }
    }

    /// Receive next message
    pub async fn recv(&mut self) -> Result<Option<Message>> {
        if let Some(stream) = &mut self.stream {
            match timeout(self.read_timeout, stream.next()).await {
                Ok(Some(Ok(msg))) => Ok(Some(msg)),
                Ok(Some(Err(e))) => Err(Error::WebSocket(e.to_string())),
                Ok(None) => Ok(None),
                Err(_) => Err(Error::WebSocket("Read timeout".into())),
            }
        } else {
            Err(Error::WebSocket("Not connected".into()))
        }
    }

    /// Close connection
    pub async fn close(&mut self) -> Result<()> {
        if let Some(stream) = &mut self.stream {
            stream
                .close(None)
                .await
                .map_err(|e| Error::WebSocket(e.to_string()))?;
        }
        self.stream = None;
        Ok(())
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }
}

/// Reconnection strategy
#[derive(Debug, Clone)]
pub struct ReconnectStrategy {
    pub max_retries: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: f64,
}

impl Default for ReconnectStrategy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_factor: 2.0,
        }
    }
}

impl ReconnectStrategy {
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay_secs = self.initial_delay.as_secs_f64()
            * self.backoff_factor.powi(attempt as i32);
        Duration::from_secs_f64(delay_secs.min(self.max_delay.as_secs_f64()))
    }
}
