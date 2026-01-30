use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("Market not found: {0}")]
    MarketNotFound(String),

    #[error("Subscription error: {0}")]
    Subscription(String),

    #[error("SBE decode error: {0}")]
    SbeDecode(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("{0}")]
    Other(String),
}

impl From<tokio_tungstenite::tungstenite::Error> for Error {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        Error::WebSocket(e.to_string())
    }
}

impl From<native_tls::Error> for Error {
    fn from(e: native_tls::Error) -> Self {
        Error::Tls(e.to_string())
    }
}

