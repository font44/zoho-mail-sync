use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::path::{Path, PathBuf};

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
    pub concurrency: Concurrency,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Concurrency {
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: usize,
    #[serde(default = "default_num_folders_to_process_concurrently")]
    pub num_folders_to_process_concurrently: usize,
    #[serde(default = "default_rate_limit_rps")]
    pub rate_limit_rps: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_base_backoff_ms")]
    pub base_backoff_ms: u64,
    #[serde(default = "default_message_list_page_size")]
    pub message_list_page_size: u32,
}

impl Default for Concurrency {
    fn default() -> Self {
        Self {
            max_concurrent_requests: default_max_concurrent_requests(),
            num_folders_to_process_concurrently: default_num_folders_to_process_concurrently(),
            rate_limit_rps: default_rate_limit_rps(),
            max_retries: default_max_retries(),
            base_backoff_ms: default_base_backoff_ms(),
            message_list_page_size: default_message_list_page_size(),
        }
    }
}

fn default_accounts_url() -> String {
    "https://accounts.zoho.com".to_string()
}
fn default_api_url() -> String {
    "https://mail.zoho.com".to_string()
}
fn default_max_concurrent_requests() -> usize {
    8
}
fn default_num_folders_to_process_concurrently() -> usize {
    100
}
fn default_rate_limit_rps() -> u32 {
    10
}
fn default_max_retries() -> u32 {
    2
}
fn default_base_backoff_ms() -> u64 {
    250
}
fn default_message_list_page_size() -> u32 {
    200
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
    pub fn token_url(&self) -> String {
        format!("{}/oauth/v2/token", self.accounts_url)
    }

    pub fn api_base(&self) -> String {
        format!("{}/api", self.api_url)
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
        accounts_url: cfg.accounts_url.trim_end_matches('/').to_string(),
        api_url: cfg.api_url.trim_end_matches('/').to_string(),
        concurrency: cfg.concurrency,
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
