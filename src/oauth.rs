use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::{BufRead, Write};

use crate::config::ResolvedConfig;
use crate::state::{ensure_state_dir, load_tokens, save_tokens, Tokens};

const ZOHO_SCOPES: &str =
    "ZohoMail.accounts.READ,ZohoMail.folders.READ,ZohoMail.messages.READ";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub async fn run_auth(cfg: &ResolvedConfig, code_arg: Option<&str>) -> Result<()> {
    let code_owned: String = match code_arg {
        Some(c) => c.trim().to_string(),
        None => {
            println!();
            println!("Self Client OAuth bootstrap");
            println!("---------------------------");
            println!("1. Open https://api-console.zoho.com/ and select your Self Client.");
            println!("2. Under 'Generate Code', enter the following scopes:");
            println!();
            println!("       {ZOHO_SCOPES}");
            println!();
            println!("3. Pick a duration (3 or 10 minutes), then 'Create'. Copy the code.");
            println!();
            print!("Paste the grant token (code) here: ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .context("reading stdin")?;
            line.trim().to_string()
        }
    };
    let code = code_owned.as_str();
    if code.is_empty() {
        return Err(anyhow!("empty grant token"));
    }

    let client = reqwest::Client::builder()
        .build()
        .context("building HTTP client")?;
    let resp = client
        .post(cfg.token_url())
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("code", code),
        ])
        .send()
        .await
        .context("POST /oauth/v2/token")?;
    let status = resp.status();
    let body = resp.text().await.context("reading token response body")?;
    if !status.is_success() {
        return Err(anyhow!("token endpoint returned {status}: {body}"));
    }
    let parsed: TokenResponse =
        serde_json::from_str(&body).with_context(|| format!("parsing token response: {body}"))?;
    if let Some(err) = parsed.error {
        return Err(anyhow!("OAuth error: {err} (full body: {body})"));
    }
    let refresh_token = parsed
        .refresh_token
        .ok_or_else(|| anyhow!("no refresh_token in response (body: {body})"))?;
    if parsed.access_token.is_none() {
        return Err(anyhow!("no access_token in response (body: {body})"));
    }

    let tokens = Tokens { refresh_token };
    ensure_state_dir(&cfg.state_dir())?;
    save_tokens(&cfg.state_dir(), &tokens)?;
    println!();
    println!("Saved refresh token to {}", cfg.state_dir().join("tokens.json").display());
    Ok(())
}

pub async fn fetch_access_token(cfg: &ResolvedConfig, client: &reqwest::Client) -> Result<String> {
    let tokens = load_tokens(&cfg.state_dir())?
        .ok_or_else(|| anyhow!("no tokens.json found; run `zoho-mail-sync auth` first"))?;
    refresh_access_token(cfg, client, &tokens.refresh_token).await
}

pub async fn refresh_access_token(
    cfg: &ResolvedConfig,
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<String> {
    let resp = client
        .post(cfg.token_url())
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("POST /oauth/v2/token (refresh)")?;
    let status = resp.status();
    let body = resp.text().await.context("reading refresh response body")?;
    if !status.is_success() {
        return Err(anyhow!("refresh failed {status}: {body}"));
    }
    let parsed: TokenResponse =
        serde_json::from_str(&body).with_context(|| format!("parsing refresh response: {body}"))?;
    if let Some(err) = parsed.error {
        return Err(anyhow!("refresh error: {err} (body: {body})"));
    }
    parsed
        .access_token
        .ok_or_else(|| anyhow!("no access_token in refresh response (body: {body})"))
}
