use reqwest::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use super::auth::KalshiAuth;
use super::models::{
    CreateOrderRequest, CreateOrderResponse, KalshiMarket, MarketsResponse,
    OrderAction, OrderSide,
};
use crate::error::{Error, Result};
use crate::constants::KALSHI_REST_URL;
use crate::exchanges::kalshi::{BatchCancelOrdersRequest, KalshiBatchCancelOrdersResponse, KalshiCancelOrder, OrderType};

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
        
        let url_path = "/trade-api/v2/markets";
        let mut url = format!("{}{}", KALSHI_REST_URL, url_path);
        
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }
        
        let auth_headers = self.auth_headers("GET", url_path)?;

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

    pub async fn _base_create_order(
        &self,
        request: CreateOrderRequest,
    ) -> Result<CreateOrderResponse> {
        let url_path = "/trade-api/v2/portfolio/orders";
        let url = format!("{}{}", KALSHI_REST_URL, url_path);
  
        let auth_headers = self.auth_headers("POST", url_path)?;

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

    pub async fn create_order(
        &self,
        ticker: &str,
        action: OrderAction,
        side: OrderSide,
        count: u64,
        price: u64,
        order_type: OrderType,
    ) -> Result<CreateOrderResponse> {
        let request: CreateOrderRequest;

        if order_type == OrderType::Market {
            request = CreateOrderRequest::market_order(
                ticker.to_string(),
                action,
                side,
                count,
                price,
            );
        } else {
            request = CreateOrderRequest::limit_order(
                ticker.to_string(),
                action,
                side,
                count,
                price,
            );
        }

        Ok(self._base_create_order(request).await?)
    }

    pub async fn batch_cancel_orders(
        &self,
        order_ids: &[&str],
    ) -> Result<KalshiBatchCancelOrdersResponse> {
        let url_path = "/trade-api/v2/portfolio/orders/batched";
        let url = format!("{}{}", KALSHI_REST_URL, url_path);
  
        let auth_headers = self.auth_headers("DELETE", url_path)?;

        let request = BatchCancelOrdersRequest {
            orders: order_ids.iter().map(|id| KalshiCancelOrder { order_id: id.to_string() }).collect(),
        };

        let resp = self
            .http
            .delete(&url)
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

        let data: KalshiBatchCancelOrdersResponse = resp.json().await.map_err(|e| Error::Http(e.to_string()))?;

        Ok(data)
    }
}
