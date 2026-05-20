use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    pub account_id: Option<String>,
}

pub fn tokens_path(state_dir: &Path) -> PathBuf {
    state_dir.join("tokens.json")
}

pub fn meta_path(state_dir: &Path) -> PathBuf {
    state_dir.join("meta.json")
}

pub fn ensure_state_dir(state_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("creating state dir {}", state_dir.display()))
}

pub fn load_tokens(state_dir: &Path) -> Result<Option<Tokens>> {
    let path = tokens_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let t = serde_json::from_str(&s)
                .with_context(|| format!("parsing {}", path.display()))?;
            Ok(Some(t))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save_tokens(state_dir: &Path, tokens: &Tokens) -> Result<()> {
    ensure_state_dir(state_dir)?;
    let path = tokens_path(state_dir);
    let json = serde_json::to_string_pretty(tokens)?;
    write_file_secret(&path, json.as_bytes())
}

pub fn load_meta(state_dir: &Path) -> Result<Meta> {
    let path = meta_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Meta::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save_meta(state_dir: &Path, meta: &Meta) -> Result<()> {
    ensure_state_dir(state_dir)?;
    let path = meta_path(state_dir);
    let json = serde_json::to_string_pretty(meta)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn write_file_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("opening {}", tmp.display()))?;
        use std::io::Write;
        f.write_all(bytes)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_file_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

