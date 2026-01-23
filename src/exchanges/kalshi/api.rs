use std::collections::HashMap;

use reqwest::Client as HttpClient;

use super::auth::KalshiAuth;
use super::models::*;
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

    fn auth_headers(&self, method: &str, path: &str) -> Result<HashMap<String, String>> {
        let headers = self.auth.generate_headers(method, path)?;
        let mut map = HashMap::new();
        map.insert("KALSHI-ACCESS-KEY".to_string(), headers.api_key);
        map.insert("KALSHI-ACCESS-TIMESTAMP".to_string(), headers.timestamp);
        map.insert("KALSHI-ACCESS-SIGNATURE".to_string(), headers.signature);
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
        let auth = self.auth_headers("GET", auth_path)?;

        let resp = self
            .http
            .get(&url)
            .header("KALSHI-ACCESS-KEY", &auth["KALSHI-ACCESS-KEY"])
            .header("KALSHI-ACCESS-TIMESTAMP", &auth["KALSHI-ACCESS-TIMESTAMP"])
            .header("KALSHI-ACCESS-SIGNATURE", &auth["KALSHI-ACCESS-SIGNATURE"])
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
}
