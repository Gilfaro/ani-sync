use async_trait::async_trait;
use color_eyre::Result;
use reqwest::{Client, Method, Response, header::HeaderMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{error, warn};

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn get_auth_url(&self) -> String;
    async fn exchange_token(&self, code: &str) -> Result<()>;
    async fn refresh_token(&self, refresh_token: &str) -> Result<()>;
}

/// Create a new reqwest client with the default user agent.
///
/// # Errors
///
/// Returns an error if the HTTP client fails to build.
pub fn create_reqwest_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?)
}

#[async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh(&self) -> Result<String>;
}

pub struct BaseClient {
    pub name: String,
    pub base_url: String,
    pub rate_limit_calls: u32,
    pub rate_limit_period: Duration,
    client: Client,
    tokens: Mutex<u32>,
    updated_at: Mutex<Instant>,
    pub refresher: Mutex<Option<Arc<dyn TokenRefresher>>>,
    refresh_lock: Mutex<()>,
}

impl BaseClient {
    /// Create a new `BaseClient`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client fails to build.
    pub fn new(
        name: &str,
        base_url: &str,
        rate_limit_calls: u32,
        rate_limit_period: Duration,
    ) -> Result<Self> {
        let client = create_reqwest_client()?;

        Ok(Self {
            name: name.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            rate_limit_calls,
            rate_limit_period,
            client,
            tokens: Mutex::new(rate_limit_calls),
            updated_at: Mutex::new(Instant::now()),
            refresher: Mutex::new(None),
            refresh_lock: Mutex::new(()),
        })
    }

    pub async fn set_refresher(&self, refresher: Arc<dyn TokenRefresher>) {
        let mut r = self.refresher.lock().await;
        *r = Some(refresher);
    }

    /// Trigger a token refresh.
    ///
    /// # Errors
    ///
    /// Returns an error if no refresher is configured or if the refresh operation fails.
    pub async fn trigger_refresh(&self) -> Result<String> {
        let refresher = self.refresher.lock().await;
        if let Some(ref r) = *refresher {
            let _lock = self.refresh_lock.lock().await;
            r.refresh().await
        } else {
            Err(color_eyre::eyre::eyre!("No refresher configured"))
        }
    }

    async fn wait_for_token(&self) {
        let mut tokens = self.tokens.lock().await;
        let mut updated_at = self.updated_at.lock().await;

        while *tokens == 0 {
            let now = Instant::now();
            let elapsed = now.duration_since(*updated_at);

            // Allow roughly rate_limit_calls per rate_limit_period
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let new_tokens = (elapsed.as_secs_f64()
                * (f64::from(self.rate_limit_calls) / self.rate_limit_period.as_secs_f64()))
                as u32;

            if new_tokens >= 1 {
                *tokens = std::cmp::min(self.rate_limit_calls, *tokens + new_tokens);
                *updated_at = now;
            }

            if *tokens == 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        *tokens -= 1;
    }

    async fn update_tokens_from_headers(&self, headers: &reqwest::header::HeaderMap) {
        if let Some(remaining) = headers.get("X-RateLimit-Remaining")
            && let Ok(r_str) = remaining.to_str()
            && let Ok(r) = r_str.parse::<u32>()
        {
            let mut tokens = self.tokens.lock().await;
            *tokens = r;
        }

        let tokens = *self.tokens.lock().await;
        if tokens < 5 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn execute_request<F>(
        &self,
        method: Method,
        endpoint: &str,
        headers: Option<HeaderMap>,
        apply_payload: F,
    ) -> Result<Response>
    where
        F: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let url = format!("{}/{}", self.base_url, endpoint.trim_start_matches('/'));

        let max_retries = 5;
        let base_delay = Duration::from_secs(1);

        for attempt in 0..max_retries {
            self.wait_for_token().await;

            let mut req = self.client.request(method.clone(), &url);
            if let Some(ref h) = headers {
                req = req.headers(h.clone());
            }
            req = apply_payload(req);

            match req.send().await {
                Ok(response) => {
                    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        let delay = if let Some(retry_after) =
                            response.headers().get(reqwest::header::RETRY_AFTER)
                        {
                            if let Ok(retry_str) = retry_after.to_str() {
                                if let Ok(retry_secs) = retry_str.parse::<u64>() {
                                    Duration::from_secs(retry_secs)
                                } else {
                                    base_delay * 2_u32.pow(attempt)
                                }
                            } else {
                                base_delay * 2_u32.pow(attempt)
                            }
                        } else {
                            base_delay * 2_u32.pow(attempt)
                        };

                        let delay = std::cmp::max(delay, Duration::from_secs(5));
                        warn!(
                            "[Rate Limit] HTTP 429 hit! Sleeping for {:?} (Attempt {}/{})",
                            delay,
                            attempt + 1,
                            max_retries
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    self.update_tokens_from_headers(response.headers()).await;

                    let is_unauthorized = response.status() == reqwest::StatusCode::UNAUTHORIZED;
                    let is_anilist_bad_request = response.status()
                        == reqwest::StatusCode::BAD_REQUEST
                        && self.name == "anilist";

                    if is_unauthorized || is_anilist_bad_request {
                        let refresher = self.refresher.lock().await;
                        if let Some(ref r) = *refresher {
                            let _lock = self.refresh_lock.lock().await;
                            match r.refresh().await {
                                Ok(new_token) => {
                                    let mut new_headers = headers.clone().unwrap_or_default();
                                    new_headers.insert(
                                        reqwest::header::AUTHORIZATION,
                                        format!("Bearer {new_token}").parse()?,
                                    );
                                    let mut req = self.client.request(method.clone(), &url);
                                    req = req.headers(new_headers);
                                    req = apply_payload(req);
                                    let retry_res = req.send().await?;
                                    return Ok(retry_res.error_for_status()?);
                                }
                                Err(e) => {
                                    error!("Token refresh failed: {e}");
                                    return Err(e);
                                }
                            }
                        }
                        return Err(color_eyre::eyre::eyre!(
                            "Authentication failed ({}) for {}. Your access token has likely expired. Please run `ani-sync auth {}` again.",
                            response.status(),
                            self.name,
                            self.name
                        ));
                    }

                    return Ok(response.error_for_status()?);
                }
                Err(e) => {
                    if attempt == max_retries - 1 {
                        return Err(e.into());
                    }
                    let delay = base_delay * 2_u32.pow(attempt);
                    error!(
                        "[Request Error] {} hit! Retrying in {:?} (Attempt {}/{})",
                        e,
                        delay,
                        attempt + 1,
                        max_retries
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(color_eyre::eyre::eyre!(
            "Failed after {} retries",
            max_retries
        ))
    }

    /// Make a request.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response status is an error.
    pub async fn request(
        &self,
        method: Method,
        endpoint: &str,
        headers: Option<HeaderMap>,
    ) -> Result<Response> {
        self.execute_request(method, endpoint, headers, |req| req)
            .await
    }

    /// Make a request with form data.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response status is an error.
    pub async fn request_with_form<T: serde::Serialize + ?Sized>(
        &self,
        method: Method,
        endpoint: &str,
        headers: Option<HeaderMap>,
        form: &T,
    ) -> Result<Response> {
        self.execute_request(method, endpoint, headers, |req| req.form(form))
            .await
    }

    /// Make a request with JSON data.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response status is an error.
    pub async fn request_with_json<T: serde::Serialize + ?Sized>(
        &self,
        method: Method,
        endpoint: &str,
        headers: Option<HeaderMap>,
        json: &T,
    ) -> Result<Response> {
        self.execute_request(method, endpoint, headers, |req| req.json(json))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_client_init() {
        let client = BaseClient::new(
            "test",
            "https://api.example.com/",
            5,
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(client.base_url, "https://api.example.com"); // Tests trailing slash removal
        assert_eq!(client.rate_limit_calls, 5);
    }
}
