pub mod folders;
pub mod messages;

use anyhow::{Context, Result, anyhow};
use governor::{
    Quota, RateLimiter,
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
};
use rand::Rng;
use reqwest::{Method, Response, StatusCode};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

use crate::config::ResolvedConfig;
use crate::oauth;

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

const ACCESS_TOKEN_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub struct Client {
    cfg: ResolvedConfig,
    http: reqwest::Client,
    access_token: Arc<Mutex<String>>,
    semaphore: Semaphore,
    rate_limiter: DirectLimiter,
    max_retries: u32,
    base_backoff_ms: u64,
}

impl Client {
    pub async fn new(cfg: ResolvedConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .gzip(true)
            .timeout(Duration::from_secs(60))
            .build()
            .context("building HTTP client")?;
        let access_token = Arc::new(Mutex::new(
            oauth::fetch_access_token(&cfg, &http).await?,
        ));
        let rps = NonZeroU32::new(cfg.concurrency.rate_limit_rps.max(1))
            .expect("rate_limit_rps clamped to >=1");
        let quota = Quota::per_second(rps);
        let max_concurrent_requests = cfg.concurrency.max_concurrent_requests.max(1);
        let max_retries = cfg.concurrency.max_retries;
        let base_backoff_ms = cfg.concurrency.base_backoff_ms;

        spawn_refresher(cfg.clone(), http.clone(), access_token.clone());

        Ok(Client {
            cfg,
            http,
            access_token,
            semaphore: Semaphore::new(max_concurrent_requests),
            rate_limiter: RateLimiter::direct(quota),
            max_retries,
            base_backoff_ms,
        })
    }

    pub fn api_base(&self) -> String {
        self.cfg.api_base()
    }

    pub fn message_list_page_size(&self) -> u32 {
        self.cfg.concurrency.message_list_page_size
    }

    pub fn num_folders_to_process_concurrently(&self) -> usize {
        self.cfg.concurrency.num_folders_to_process_concurrently.max(1)
    }

    async fn rate_limit(&self) {
        self.rate_limiter.until_ready().await;
    }

    async fn current_token(&self) -> String {
        self.access_token.lock().await.clone()
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let bytes = self.get_bytes(url).await?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing JSON from {url}"))
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let mut attempt: u32 = 0;
        loop {
            let outcome = {
                let _permit = self.semaphore.acquire().await.context("semaphore")?;
                self.rate_limit().await;
                let token = self.current_token().await;
                let resp = self
                    .http
                    .request(Method::GET, url)
                    .header("Authorization", format!("Zoho-oauthtoken {token}"))
                    .header("Accept", "application/json")
                    .send()
                    .await;
                handle_response(resp).await
            };
            match outcome {
                ResponseOutcome::Ok(body) => return Ok(body),
                ResponseOutcome::Retry { delay } => {
                    if attempt >= self.max_retries {
                        return Err(anyhow!("retry limit hit for {url}"));
                    }
                    let wait = delay.unwrap_or_else(|| backoff_for(self.base_backoff_ms, attempt));
                    tracing::warn!("retrying {url} after {:?} (attempt {})", wait, attempt + 1);
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                ResponseOutcome::Fatal(e) => return Err(e),
            }
        }
    }
}

fn spawn_refresher(
    cfg: ResolvedConfig,
    http: reqwest::Client,
    access_token: Arc<Mutex<String>>,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(ACCESS_TOKEN_REFRESH_INTERVAL);
        tick.tick().await;
        loop {
            tick.tick().await;
            match oauth::fetch_access_token(&cfg, &http).await {
                Ok(new) => {
                    *access_token.lock().await = new;
                    tracing::debug!("refreshed access token");
                }
                Err(e) => tracing::warn!("background token refresh failed: {e:#}"),
            }
        }
    });
}

enum ResponseOutcome {
    Ok(Vec<u8>),
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
                let url = r.url().clone();
                let body = r.text().await.unwrap_or_default();
                ResponseOutcome::Fatal(anyhow!(
                    "401 from {url}: background token refresher must have failed (body: {body})"
                ))
            } else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                let url = r.url().clone();
                let delay = match r.headers().get("retry-after") {
                    None => None,
                    Some(v) => match v.to_str().ok().and_then(|s| s.parse::<u64>().ok()) {
                        Some(n) => Some(Duration::from_secs(n)),
                        None => {
                            let raw = v.to_str().unwrap_or("<non-utf8>").to_string();
                            return ResponseOutcome::Fatal(anyhow!(
                                "unsupported Retry-After header value {raw:?} from {url}: \
                                 only integer-seconds form is supported"
                            ));
                        }
                    },
                };
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

fn backoff_for(base_ms: u64, attempt: u32) -> Duration {
    let exp = base_ms.saturating_mul(1 << attempt.min(5));
    let factor: f64 = rand::thread_rng().gen_range(0.5..1.5);
    let jittered = (exp as f64 * factor) as u64;
    Duration::from_millis(jittered.max(1))
}
