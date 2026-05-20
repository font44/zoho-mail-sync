use anyhow::{Context, Result, anyhow};
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::config::ResolvedConfig;
use crate::maildir::{self, Flags};
use crate::state;
use crate::zoho::{
    Client,
    folders::{Folder, discover_account_id, list_account_ids, list_folders},
    messages::{RemoteMessage, fetch_original_message, list_folder_messages},
};

pub async fn run(cfg: ResolvedConfig) -> Result<()> {
    state::ensure_state_dir(&cfg.state_dir())?;
    let client = Arc::new(Client::new(cfg.clone()).await?);
    let mut meta = state::load_meta(&cfg.state_dir())?;

    let account_id = resolve_account_id(&client, &mut meta, &cfg).await?;
    tracing::info!(account_id = %account_id, "using Zoho account");

    let folders = list_folders(&client, &account_id).await?;
    tracing::info!(count = folders.len(), "listed remote folders");
    {
        let data_dir = cfg.data_dir.clone();
        let folders_clone = folders.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            for f in &folders_clone {
                maildir::ensure_folder(&data_dir, &f.maildir_name)?;
            }
            Ok(())
        })
        .await
        .context("ensure_folder task panicked")??;
    }

    let local = {
        let data_dir = cfg.data_dir.clone();
        tokio::task::spawn_blocking(move || maildir::scan_data_dir(&data_dir))
            .await
            .context("scan_data_dir task panicked")??
    };
    tracing::info!(count = local.len(), "scanned local maildir");

    let (remote, enum_errors) = enumerate_remote(&client, &account_id, &folders).await;
    if !enum_errors.is_empty() {
        for e in &enum_errors {
            tracing::error!("{e:#}");
        }
        anyhow::bail!(
            "{} folder enumeration(s) failed; refusing to diff against partial remote state",
            enum_errors.len()
        );
    }
    tracing::info!(count = remote.len(), "enumerated remote messages");

    let folder_id_to_maildir: HashMap<String, String> = folders
        .iter()
        .map(|f| (f.folder_id.clone(), f.maildir_name.clone()))
        .collect();

    let (to_fetch, mut errors) = {
        let data_dir = cfg.data_dir.clone();
        tokio::task::spawn_blocking(move || diff_local_remote(&data_dir, &remote, &local, &folder_id_to_maildir))
            .await
            .context("diff task panicked")?
    };

    let fetch_count = to_fetch.len();
    tracing::info!(new = fetch_count, "starting fetches");
    let fetch_errors = apply_fetches(client.clone(), &account_id, &cfg.data_dir, to_fetch).await;
    let fetch_failed = fetch_errors.len();
    errors.extend(fetch_errors);

    let stale_errors = {
        let data_dir = cfg.data_dir.clone();
        let folders_clone = folders.clone();
        tokio::task::spawn_blocking(move || cleanup_stale_dirs(&data_dir, &folders_clone))
            .await
            .context("cleanup_stale_dirs task panicked")??
    };
    let stale_failed = stale_errors.len();
    errors.extend(stale_errors);

    state::save_meta(&cfg.state_dir(), &meta)?;

    if !errors.is_empty() {
        for e in &errors {
            tracing::error!("{e:#}");
        }
        anyhow::bail!(
            "{} operation(s) failed ({} fetch, {} stale-dir, {} other)",
            errors.len(),
            fetch_failed,
            stale_failed,
            errors.len() - fetch_failed - stale_failed
        );
    }
    tracing::info!("sync complete");
    Ok(())
}

fn diff_local_remote(
    data_dir: &std::path::Path,
    remote: &HashMap<String, RemoteMessage>,
    local: &HashMap<String, crate::maildir::LocalEntry>,
    folder_id_to_maildir: &HashMap<String, String>,
) -> (Vec<(String, String, Flags)>, Vec<anyhow::Error>) {
    let mut errors: Vec<anyhow::Error> = Vec::new();
    let mut to_fetch: Vec<(String, String, Flags)> = Vec::new();

    for (mid, rmsg) in remote {
        let target = match folder_id_to_maildir.get(&rmsg.folder_id) {
            Some(n) => n.clone(),
            None => {
                tracing::warn!(
                    folder_id = %rmsg.folder_id,
                    message_id = %mid,
                    "remote message in unknown folder; skipping"
                );
                continue;
            }
        };
        let flags = flags_from_remote(rmsg);
        match local.get(mid) {
            None => to_fetch.push((mid.clone(), target, flags)),
            Some(le) if le.maildir_name != target => {
                if let Err(e) =
                    maildir::move_to_folder(data_dir, &le.maildir_name, &target, mid, flags)
                {
                    errors.push(e);
                }
            }
            Some(le) if le.flags != flags => {
                if let Err(e) = maildir::set_flags(data_dir, &target, mid, flags) {
                    errors.push(e);
                }
            }
            Some(_) => {}
        }
    }

    for (mid, le) in local {
        if !remote.contains_key(mid)
            && let Err(e) = maildir::delete(data_dir, &le.maildir_name, mid)
        {
            errors.push(e);
        }
    }

    (to_fetch, errors)
}

async fn resolve_account_id(
    client: &Client,
    meta: &mut state::Meta,
    cfg: &ResolvedConfig,
) -> Result<String> {
    match meta.account_id.clone() {
        Some(saved) => {
            let ids = list_account_ids(client).await?;
            if !ids.iter().any(|id| id == &saved) {
                return Err(anyhow!(
                    "saved account_id {saved} not present in Zoho /accounts response (got {ids:?}); \
                     the account was deleted or revoked, or your tokens are for a different user"
                ));
            }
            Ok(saved)
        }
        None => {
            let id = discover_account_id(client).await?;
            meta.account_id = Some(id.clone());
            state::save_meta(&cfg.state_dir(), meta)?;
            Ok(id)
        }
    }
}

async fn enumerate_remote(
    client: &Arc<Client>,
    account_id: &str,
    folders: &[Folder],
) -> (HashMap<String, RemoteMessage>, Vec<anyhow::Error>) {
    let parallelism = client.num_folders_to_process_concurrently();
    let stream = futures::stream::iter(folders.iter().map(|f| {
        let client = client.clone();
        let account_id = account_id.to_string();
        let maildir_name = f.maildir_name.clone();
        let folder_id = f.folder_id.clone();
        async move {
            let msgs = list_folder_messages(&client, &account_id, &folder_id)
                .await
                .with_context(|| format!("enumerating folder {maildir_name}"))?;
            tracing::info!(folder = %maildir_name, count = msgs.len(), "enumerated folder");
            Ok::<_, anyhow::Error>(msgs)
        }
    }))
    .buffer_unordered(parallelism);

    let results: Vec<Result<Vec<RemoteMessage>>> = stream.collect().await;
    let mut out: HashMap<String, RemoteMessage> = HashMap::new();
    let mut errors: Vec<anyhow::Error> = Vec::new();
    for r in results {
        match r {
            Ok(msgs) => {
                for m in msgs {
                    if out.insert(m.message_id.clone(), m).is_some() {
                        tracing::warn!("duplicate message_id seen across folders; last folder wins");
                    }
                }
            }
            Err(e) => errors.push(e),
        }
    }
    (out, errors)
}

fn flags_from_remote(m: &RemoteMessage) -> Flags {
    Flags {
        seen: m.read,
        flagged: m.important,
    }
}

async fn apply_fetches(
    client: Arc<Client>,
    account_id: &str,
    data_dir: &std::path::Path,
    items: Vec<(String, String, Flags)>,
) -> Vec<anyhow::Error> {
    let mut errors = Vec::new();
    if items.is_empty() {
        return errors;
    }
    let pb = indicatif::ProgressBar::new(items.len() as u64);
    pb.set_style(
        indicatif::ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} {msg}").unwrap(),
    );

    let mut futures = FuturesUnordered::new();
    for (mid, maildir_name, flags) in items {
        let client = client.clone();
        let account_id = account_id.to_string();
        let data_dir = data_dir.to_path_buf();
        futures.push(async move {
            let bytes = fetch_original_message(&client, &account_id, &mid)
                .await
                .with_context(|| format!("fetching message {mid}"))?;
            tokio::task::spawn_blocking(move || {
                maildir::write_message(&data_dir, &maildir_name, &mid, flags, &bytes)
            })
            .await
            .context("write_message task panicked")??;
            Ok::<_, anyhow::Error>(())
        });
    }
    while let Some(res) = futures.next().await {
        pb.inc(1);
        if let Err(e) = res {
            errors.push(e);
        }
    }
    pb.finish_with_message("done");
    errors
}

fn cleanup_stale_dirs(data_dir: &std::path::Path, folders: &[Folder]) -> Result<Vec<anyhow::Error>> {
    let active: HashSet<&str> = folders.iter().map(|f| f.maildir_name.as_str()).collect();
    let local_folders = maildir::list_local_folders(data_dir)?;
    let mut errors = Vec::new();
    for name in local_folders {
        if active.contains(name.as_str()) {
            continue;
        }
        match maildir::rmdir_if_empty(data_dir, &name) {
            Ok(true) => tracing::info!(folder = %name, "removed stale empty maildir"),
            Ok(false) => tracing::warn!(folder = %name, "stale maildir not empty; leaving in place"),
            Err(e) => errors.push(e),
        }
    }
    Ok(errors)
}
