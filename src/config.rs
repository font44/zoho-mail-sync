use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT_FETCH_PARALLELISM: usize = 8;
pub const DEFAULT_MAX_CONCURRENT: usize = 8;
pub const DEFAULT_RATE_LIMIT_RPS: u32 = 10;
pub const DEFAULT_RATE_LIMIT_BURST: u32 = 20;
pub const DEFAULT_MAX_RETRIES: u32 = 5;
pub const DEFAULT_BASE_BACKOFF_MS: u64 = 250;
pub const DEFAULT_PAGE_LIMIT: u32 = 200;

const ENV_CLIENT_ID: &str = "ZOHO_CLIENT_ID";
const ENV_CLIENT_SECRET: &str = "ZOHO_CLIENT_SECRET";

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    #[serde(default = "default_accounts_url")]
    pub accounts_url: String,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default)]
    pub concurrency: ConcurrencyConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConcurrencyConfig {
    #[serde(default)]
    pub fetch_parallelism: Option<usize>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    #[serde(default)]
    pub rate_limit_rps: Option<u32>,
    #[serde(default)]
    pub rate_limit_burst: Option<u32>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub base_backoff_ms: Option<u64>,
    #[serde(default)]
    pub page_limit: Option<u32>,
}

fn default_accounts_url() -> String {
    "https://accounts.zoho.com".to_string()
}

fn default_api_url() -> String {
    "https://mail.zoho.com".to_string()
}

#[derive(Debug, Clone)]
pub struct Concurrency {
    pub fetch_parallelism: usize,
    pub max_concurrent: usize,
    pub rate_limit_rps: u32,
    pub rate_limit_burst: u32,
    pub max_retries: u32,
    pub base_backoff_ms: u64,
    pub page_limit: u32,
}

impl Concurrency {
    fn from_config(c: &ConcurrencyConfig) -> Self {
        Self {
            fetch_parallelism: c.fetch_parallelism.unwrap_or(DEFAULT_FETCH_PARALLELISM),
            max_concurrent: c.max_concurrent.unwrap_or(DEFAULT_MAX_CONCURRENT),
            rate_limit_rps: c.rate_limit_rps.unwrap_or(DEFAULT_RATE_LIMIT_RPS),
            rate_limit_burst: c.rate_limit_burst.unwrap_or(DEFAULT_RATE_LIMIT_BURST),
            max_retries: c.max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
            base_backoff_ms: c.base_backoff_ms.unwrap_or(DEFAULT_BASE_BACKOFF_MS),
            page_limit: c.page_limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub client_id: String,
    pub client_secret: String,
    pub data_dir: PathBuf,
    pub accounts_url: String,
    pub api_url: String,
    pub concurrency: Concurrency,
}

impl ResolvedConfig {
    pub fn accounts_host(&self) -> &str {
        self.accounts_url.trim_end_matches('/')
    }

    pub fn api_host(&self) -> &str {
        self.api_url.trim_end_matches('/')
    }

    pub fn token_url(&self) -> String {
        format!("{}/oauth/v2/token", self.accounts_host())
    }

    pub fn state_dir(&self) -> PathBuf {
        self.data_dir.join(".zoho-mail-sync")
    }
}

pub fn load(config_path: Option<&Path>, data_dir_override: Option<&Path>) -> Result<ResolvedConfig> {
    let cfg = read_toml(config_path)?;

    let client_id = require_env(ENV_CLIENT_ID)?;
    let client_secret = require_env(ENV_CLIENT_SECRET)?;

    let data_dir = data_dir_override
        .map(Path::to_path_buf)
        .or(cfg.data_dir)
        .unwrap_or_else(|| std::env::current_dir().expect("CWD"));

    Ok(ResolvedConfig {
        client_id,
        client_secret,
        data_dir,
        accounts_url: cfg.accounts_url,
        api_url: cfg.api_url,
        concurrency: Concurrency::from_config(&cfg.concurrency),
    })
}

fn read_toml(config_path: Option<&Path>) -> Result<Config> {
    match config_path {
        Some(p) => {
            let raw = std::fs::read_to_string(p)
                .with_context(|| format!("reading config {}", p.display()))?;
            toml::from_str(&raw).with_context(|| format!("parsing config {}", p.display()))
        }
        None => {
            let default_path = PathBuf::from("zoho-mail-sync.toml");
            match std::fs::read_to_string(&default_path) {
                Ok(raw) => toml::from_str(&raw)
                    .with_context(|| format!("parsing config {}", default_path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(toml::from_str("").expect("empty TOML deserializes to defaults"))
                }
                Err(e) => Err(e).with_context(|| format!("reading config {}", default_path.display())),
            }
        }
    }
}

fn require_env(name: &str) -> Result<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        Ok(_) => Err(anyhow!("{name} is set but empty")),
        Err(_) => Err(anyhow!(
            "{name} is not set; export it in your shell, .envrc, or systemd EnvironmentFile"
        )),
    }
}
