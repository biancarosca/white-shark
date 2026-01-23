use base64::Engine;
use pem::parse;
use rand::rngs::OsRng;
use rsa::{
    pkcs1::DecodeRsaPrivateKey,
    pkcs8::DecodePrivateKey,
    pss::BlindedSigningKey,
    signature::{RandomizedSigner, SignatureEncoding},
    RsaPrivateKey,
};
use sha2::Sha256;

use crate::error::{Error, Result};

pub struct KalshiAuth {
    private_key: RsaPrivateKey,
    api_key_id: String,
}

impl KalshiAuth {
    pub fn from_file(api_key_id: &str, private_key_path: &str) -> Result<Self> {
        let key_data = std::fs::read(private_key_path)
            .map_err(|e| Error::Auth(format!("Failed to read private key: {}", e)))?;

        let pem_data = parse(&key_data)
            .map_err(|e| Error::Auth(format!("Failed to parse PEM: {}", e)))?;

        let private_key = match pem_data.tag().to_string().as_str() {
            "PRIVATE KEY" => {
                RsaPrivateKey::from_pkcs8_der(pem_data.contents())
                    .map_err(|e| Error::Auth(format!("Failed to parse PKCS#8 key: {}", e)))?
            }
            "RSA PRIVATE KEY" => {
                RsaPrivateKey::from_pkcs1_der(pem_data.contents())
                    .map_err(|e| Error::Auth(format!("Failed to parse PKCS#1 key: {}", e)))?
            }
            _ => {
                RsaPrivateKey::from_pkcs8_der(pem_data.contents())
                    .or_else(|_| RsaPrivateKey::from_pkcs1_der(pem_data.contents()))
                    .map_err(|e| Error::Auth(format!("Failed to parse key: {}", e)))?
            }
        };

        Ok(Self {
            private_key,
            api_key_id: api_key_id.to_string(),
        })
    }

    pub fn api_key_id(&self) -> &str {
        &self.api_key_id
    }

    pub fn sign(&self, message: &str) -> Result<String> {
        let mut rng = OsRng;
        let signing_key = BlindedSigningKey::<Sha256>::new(self.private_key.clone());
        let signature = signing_key.sign_with_rng(&mut rng, message.as_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
        Ok(encoded)
    }

    pub fn generate_headers(&self, method: &str, path: &str) -> Result<AuthHeaders> {
        let timestamp = chrono::Utc::now().timestamp_millis().to_string();
        let message = format!("{}{}{}", timestamp, method, path);
        let signature = self.sign(&message)?;

        Ok(AuthHeaders {
            api_key: self.api_key_id.clone(),
            timestamp,
            signature,
        })
    }

    pub fn generate_ws_headers(&self) -> Result<AuthHeaders> {
        self.generate_headers("GET", "/trade-api/ws/v2")
    }
}

#[derive(Debug, Clone)]
pub struct AuthHeaders {
    pub api_key: String,
    pub timestamp: String,
    pub signature: String,
}

impl AuthHeaders {
    pub fn to_header_tuples(&self) -> Vec<(&'static str, String)> {
        vec![
            ("KALSHI-ACCESS-KEY", self.api_key.clone()),
            ("KALSHI-ACCESS-TIMESTAMP", self.timestamp.clone()),
            ("KALSHI-ACCESS-SIGNATURE", self.signature.clone()),
        ]
    }
}

