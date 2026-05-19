pub mod folders;
pub mod messages;

use anyhow::{anyhow, Context, Result};
use governor::{
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use reqwest::{Method, Response, StatusCode};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

use crate::config::ResolvedConfig;
use crate::oauth;

const MAX_CONCURRENT: usize = 8;
const RATE_LIMIT_RPS: u32 = 10;
const RATE_LIMIT_BURST: u32 = 20;
const MAX_RETRIES: u32 = 5;
const BASE_BACKOFF_MS: u64 = 250;

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

pub struct Client {
    cfg: ResolvedConfig,
    http: reqwest::Client,
    access_token: Mutex<String>,
    semaphore: Arc<Semaphore>,
    rate_limiter: Arc<DirectLimiter>,
}

impl Client {
    pub async fn new(cfg: ResolvedConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .gzip(true)
            .timeout(Duration::from_secs(60))
            .build()
            .context("building HTTP client")?;
        let access_token = oauth::fetch_access_token(&cfg, &http).await?;
        let quota = Quota::per_second(NonZeroU32::new(RATE_LIMIT_RPS).unwrap())
            .allow_burst(NonZeroU32::new(RATE_LIMIT_BURST).unwrap());
        Ok(Client {
            cfg,
            http,
            access_token: Mutex::new(access_token),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
        })
    }

    pub fn api_base(&self) -> String {
        format!("{}/api", self.cfg.api_host())
    }

    async fn rate_limit(&self) {
        self.rate_limiter.until_ready().await;
    }

    async fn current_token(&self) -> String {
        self.access_token.lock().await.clone()
    }

    async fn re_refresh(&self) -> Result<()> {
        let new = oauth::fetch_access_token(&self.cfg, &self.http).await?;
        *self.access_token.lock().await = new;
        Ok(())
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let bytes = self.get_bytes(url).await?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing JSON from {url}"))
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let _permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .context("semaphore")?;
        let mut attempt: u32 = 0;
        let mut refreshed_once = false;
        loop {
            self.rate_limit().await;
            let token = self.current_token().await;
            let resp = self
                .http
                .request(Method::GET, url)
                .header(
                    "Authorization",
                    format!("Zoho-oauthtoken {token}"),
                )
                .header("Accept", "application/json")
                .send()
                .await;
            match handle_response(resp).await {
                ResponseOutcome::Ok(body) => return Ok(body),
                ResponseOutcome::Unauthorized => {
                    if refreshed_once {
                        return Err(anyhow!("401 from {url} after token refresh"));
                    }
                    tracing::warn!("got 401 on {url}, refreshing access token");
                    self.re_refresh().await?;
                    refreshed_once = true;
                    continue;
                }
                ResponseOutcome::Retry { delay } => {
                    if attempt >= MAX_RETRIES {
                        return Err(anyhow!("retry limit hit for {url}"));
                    }
                    let wait = delay.unwrap_or_else(|| backoff_for(attempt));
                    tracing::warn!("retrying {url} after {:?} (attempt {})", wait, attempt + 1);
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                ResponseOutcome::Fatal(e) => return Err(e),
            }
        }
    }
}

enum ResponseOutcome {
    Ok(Vec<u8>),
    Unauthorized,
    Retry { delay: Option<Duration> },
    Fatal(anyhow::Error),
}

async fn handle_response(resp: reqwest::Result<Response>) -> ResponseOutcome {
    match resp {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                match r.bytes().await {
                    Ok(b) => ResponseOutcome::Ok(b.to_vec()),
                    Err(e) => ResponseOutcome::Fatal(anyhow!("reading body: {e}")),
                }
            } else if status == StatusCode::UNAUTHORIZED {
                ResponseOutcome::Unauthorized
            } else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                let delay = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs);
                ResponseOutcome::Retry { delay }
            } else {
                let url = r.url().clone();
                let body = r.text().await.unwrap_or_default();
                ResponseOutcome::Fatal(anyhow!("{status} from {url}: {body}"))
            }
        }
        Err(e) if e.is_timeout() || e.is_connect() => ResponseOutcome::Retry { delay: None },
        Err(e) => ResponseOutcome::Fatal(anyhow!("HTTP error: {e}")),
    }
}

fn backoff_for(attempt: u32) -> Duration {
    let ms = BASE_BACKOFF_MS.saturating_mul(1 << attempt.min(5));
    Duration::from_millis(ms)
}
