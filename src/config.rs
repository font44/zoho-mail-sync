use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    #[serde(default = "default_accounts_url")]
    pub accounts_url: String,
    #[serde(default = "default_api_url")]
    pub api_url: String,
}

fn default_accounts_url() -> String {
    "https://accounts.zoho.com".to_string()
}

fn default_api_url() -> String {
    "https://mail.zoho.com".to_string()
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub client_id: String,
    pub client_secret: String,
    pub data_dir: PathBuf,
    pub accounts_url: String,
    pub api_url: String,
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
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from("zoho-mail-sync.toml"),
    };

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading config {}", path.display()))?;
    let cfg: Config = toml::from_str(&raw)
        .with_context(|| format!("parsing config {}", path.display()))?;

    let data_dir = data_dir_override
        .map(Path::to_path_buf)
        .or(cfg.data_dir)
        .unwrap_or_else(|| std::env::current_dir().expect("CWD"));

    Ok(ResolvedConfig {
        client_id: cfg.client_id,
        client_secret: cfg.client_secret,
        data_dir,
        accounts_url: cfg.accounts_url,
        api_url: cfg.api_url,
    })
}

