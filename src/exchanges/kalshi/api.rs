use reqwest::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use super::auth::KalshiAuth;
use super::models::{
    CreateOrderRequest, CreateOrderResponse, KalshiMarket, MarketsResponse,
    OrderAction, OrderSide,
};
use crate::error::{Error, Result};
use crate::constants::KALSHI_REST_URL;

pub struct KalshiApi {
    http: HttpClient,
    auth: std::sync::Arc<KalshiAuth>,
}

impl KalshiApi {
    pub fn new(auth: std::sync::Arc<KalshiAuth>) -> Self {
        Self {
            http: HttpClient::new(),
            auth,
        }
    }

    fn auth_headers(&self, method: &str, path: &str) -> Result<HeaderMap> {
        let headers = self.auth.generate_headers(method, path)?;
        let mut map = HeaderMap::new();
        map.insert(
            HeaderName::from_static("kalshi-access-key"),
            HeaderValue::from_str(&headers.api_key).map_err(|e| Error::Http(e.to_string()))?,
        );
        map.insert(
            HeaderName::from_static("kalshi-access-timestamp"),
            HeaderValue::from_str(&headers.timestamp).map_err(|e| Error::Http(e.to_string()))?,
        );
        map.insert(
            HeaderName::from_static("kalshi-access-signature"),
            HeaderValue::from_str(&headers.signature).map_err(|e| Error::Http(e.to_string()))?,
        );
        Ok(map)
    }

    pub async fn fetch_markets(
        &self,
        status: Option<&str>,
        series_ticker: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<MarketsResponse> {
        
        let mut params = vec![];
        if let Some(s) = status {
            params.push(format!("status={}", s));
        }
        if let Some(ticker) = series_ticker {
            params.push(format!("series_ticker={}", ticker));
        }
        if let Some(c) = cursor {
            params.push(format!("cursor={}", c));
        }
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        
        let url_path = "/markets";
        let mut url = format!("{}{}", KALSHI_REST_URL, url_path);
        
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }
        
        let auth_path = "/trade-api/v2/markets";
        let auth_headers = self.auth_headers("GET", auth_path)?;

        let resp = self
            .http
            .get(&url)
            .headers(auth_headers)
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("HTTP {}: {}", status, body)));
        }

        let data: MarketsResponse = resp
            .json()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        println!("data {:?}", data);

        Ok(data)
    }

    pub async fn fetch_market_by_ticker(
        &self,
        series_ticker: &str,
        status: Option<&str>,
    ) -> Result<Vec<KalshiMarket>> {
        let mut all_markets = Vec::new();
        let mut cursor = None;

        loop {
            let resp = self
                .fetch_markets(status, Some(series_ticker), cursor.as_deref(), None)
                .await?;

            all_markets.extend(resp.markets);

            match resp.cursor {
                Some(c) => {
                    if c.is_empty() {
                        break;
                    }
                    cursor = Some(c)
                },
                None => break
            }
        }

        Ok(all_markets)
    }

    pub async fn get_markets_for_tickers(&self, tickers: &[&str]) -> Result<Vec<KalshiMarket>> {
        let mut all_markets = Vec::new();
        for ticker in tickers {
            let markets = self.fetch_market_by_ticker(ticker, Some("open")).await?;
            if markets.is_empty() {
                return Err(Error::Other(format!("No markets found for ticker: {}", ticker)));
            }
            if markets.len() > 1 {
                return Err(Error::Other(format!("Multiple markets found for ticker: {}", ticker)));
            }
            all_markets.push(markets[0].clone());
        }
        Ok(all_markets)
    }

    /// Create a market order on Kalshi
    /// 
    /// # Arguments
    /// * `ticker` - The market ticker (e.g., "KXBTC-25JAN31-B55000")
    /// * `action` - Buy or Sell
    /// * `side` - Yes or No
    /// * `count` - Number of contracts
    pub async fn create_market_order(
        &self,
        ticker: &str,
        action: OrderAction,
        side: OrderSide,
        count: i64,
    ) -> Result<CreateOrderResponse> {
        let request = CreateOrderRequest::market_order(
            ticker.to_string(),
            action,
            side,
            count,
        );

        let url_path = "/portfolio/orders";
        let url = format!("{}{}", KALSHI_REST_URL, url_path);
        let auth_path = "/trade-api/v2/portfolio/orders";
        let auth_headers = self.auth_headers("POST", auth_path)?;

        let resp = self
            .http
            .post(&url)
            .headers(auth_headers)
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("HTTP {}: {}", status, body)));
        }

        let data: CreateOrderResponse = resp
            .json()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;

        Ok(data)
    }
}
