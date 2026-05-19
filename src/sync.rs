use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::config::ResolvedConfig;
use crate::maildir::{self, Flags};
use crate::state;
use crate::zoho::{
    folders::{discover_account_id, list_folders, Folder},
    messages::{fetch_original_message, list_folder_messages, RemoteMessage},
    Client,
};

pub async fn run(cfg: ResolvedConfig) -> Result<()> {
    state::ensure_state_dir(&cfg.state_dir())?;
    let client = Arc::new(Client::new(cfg.clone()).await?);
    let mut meta = state::load_meta(&cfg.state_dir())?;

    let account_id = match meta.account_id.clone() {
        Some(id) => id,
        None => {
            let id = discover_account_id(&client).await?;
            meta.account_id = Some(id.clone());
            state::save_meta(&cfg.state_dir(), &meta)?;
            id
        }
    };
    tracing::info!(account_id = %account_id, "using Zoho account");

    let folders = list_folders(&client, &account_id).await?;
    tracing::info!(count = folders.len(), "listed remote folders");
    meta.folder_map.clear();
    for f in &folders {
        meta.folder_map.insert(f.folder_id.clone(), f.maildir_name.clone());
        maildir::ensure_folder(&cfg.data_dir, &f.maildir_name)?;
    }

    let local = maildir::scan_data_dir(&cfg.data_dir)?;
    tracing::info!(count = local.len(), "scanned local maildir");

    let remote = enumerate_remote(&client, &account_id, &folders).await?;
    tracing::info!(count = remote.len(), "enumerated remote messages");

    let folder_id_to_maildir: HashMap<String, String> = folders
        .iter()
        .map(|f| (f.folder_id.clone(), f.maildir_name.clone()))
        .collect();
    let folder_type_map: HashMap<String, Option<String>> = folders
        .iter()
        .map(|f| (f.folder_id.clone(), f.folder_type.clone()))
        .collect();

    let mut to_fetch: Vec<(String, String, Flags)> = Vec::new();
    let mut to_set_flags: Vec<(String, String, Flags)> = Vec::new();
    let mut to_move: Vec<(String, String, String, Flags)> = Vec::new();
    let mut to_delete: Vec<(String, String)> = Vec::new();

    for (mid, rmsg) in &remote {
        let target_folder_id = &rmsg.folder_id;
        let target_maildir_name = match folder_id_to_maildir.get(target_folder_id) {
            Some(n) => n.clone(),
            None => {
                tracing::warn!(
                    folder_id = %target_folder_id,
                    message_id = %mid,
                    "remote message in unknown folder; skipping"
                );
                continue;
            }
        };
        let ftype = folder_type_map
            .get(target_folder_id)
            .cloned()
            .flatten();
        let flags = flags_from_remote(rmsg, ftype.as_deref());
        match local.get(mid) {
            None => {
                to_fetch.push((mid.clone(), target_maildir_name, flags));
            }
            Some(le) => {
                if le.maildir_name != target_maildir_name {
                    to_move.push((le.maildir_name.clone(), target_maildir_name, mid.clone(), flags));
                } else if le.flags != flags {
                    to_set_flags.push((target_maildir_name, mid.clone(), flags));
                }
            }
        }
    }

    for (mid, le) in &local {
        if !remote.contains_key(mid) {
            to_delete.push((le.maildir_name.clone(), mid.clone()));
        }
    }

    tracing::info!(
        new = to_fetch.len(),
        flag_changes = to_set_flags.len(),
        moves = to_move.len(),
        deletes = to_delete.len(),
        "diff complete"
    );

    apply_flag_changes(&cfg.data_dir, &to_set_flags)?;
    apply_moves(&cfg.data_dir, &to_move)?;
    apply_fetches(
        client.clone(),
        &account_id,
        &cfg.data_dir,
        cfg.concurrency.fetch_parallelism,
        &to_fetch,
    )
    .await?;
    apply_deletes(&cfg.data_dir, &to_delete)?;

    meta.last_sync_unix = Some(unix_now());
    state::save_meta(&cfg.state_dir(), &meta)?;
    tracing::info!("sync complete");
    Ok(())
}

async fn enumerate_remote(
    client: &Client,
    account_id: &str,
    folders: &[Folder],
) -> Result<HashMap<String, RemoteMessage>> {
    let mut out: HashMap<String, RemoteMessage> = HashMap::new();
    for f in folders {
        let msgs = list_folder_messages(client, account_id, &f.folder_id).await?;
        tracing::info!(folder = %f.maildir_name, count = msgs.len(), "enumerated folder");
        for m in msgs {
            out.insert(m.message_id.clone(), m);
        }
    }
    Ok(out)
}

fn flags_from_remote(m: &RemoteMessage, folder_type: Option<&str>) -> Flags {
    let ftype_lc = folder_type.map(|s| s.to_lowercase());
    let in_drafts = matches!(ftype_lc.as_deref(), Some("drafts"));
    let in_trash = matches!(ftype_lc.as_deref(), Some("trash"));
    Flags {
        seen: m.read,
        flagged: m.important,
        draft: in_drafts,
        trashed: in_trash,
    }
}

fn apply_flag_changes(data_dir: &std::path::Path, items: &[(String, String, Flags)]) -> Result<()> {
    for (folder, mid, flags) in items {
        maildir::set_flags(data_dir, folder, mid, *flags)?;
    }
    Ok(())
}

fn apply_moves(
    data_dir: &std::path::Path,
    items: &[(String, String, String, Flags)],
) -> Result<()> {
    for (from, to, mid, flags) in items {
        maildir::move_to_folder(data_dir, from, to, mid, *flags)?;
    }
    Ok(())
}

async fn apply_fetches(
    client: Arc<Client>,
    account_id: &str,
    data_dir: &std::path::Path,
    parallelism: usize,
    items: &[(String, String, Flags)],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let parallelism = parallelism.max(1);
    let pb = indicatif::ProgressBar::new(items.len() as u64);
    pb.set_style(
        indicatif::ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap(),
    );

    let mut futures = FuturesUnordered::new();
    let mut iter = items.iter();
    let mut in_flight = 0usize;
    let mut errors: Vec<anyhow::Error> = Vec::new();

    loop {
        while in_flight < parallelism {
            match iter.next() {
                Some((mid, maildir_name, flags)) => {
                    let client = client.clone();
                    let mid = mid.clone();
                    let maildir_name = maildir_name.clone();
                    let flags = *flags;
                    let account_id = account_id.to_string();
                    let data_dir = data_dir.to_path_buf();
                    futures.push(tokio::spawn(async move {
                        fetch_one(client, account_id, mid, data_dir, maildir_name, flags).await
                    }));
                    in_flight += 1;
                }
                None => break,
            }
        }
        if in_flight == 0 {
            break;
        }
        if let Some(joined) = futures.next().await {
            in_flight -= 1;
            match joined {
                Ok(Ok(())) => pb.inc(1),
                Ok(Err(e)) => {
                    pb.inc(1);
                    errors.push(e);
                }
                Err(e) => {
                    pb.inc(1);
                    errors.push(anyhow::anyhow!("join error: {e}"));
                }
            }
        }
    }
    pb.finish_with_message("done");
    if !errors.is_empty() {
        for e in &errors {
            tracing::error!("{e:#}");
        }
        anyhow::bail!("{} message fetches failed", errors.len());
    }
    Ok(())
}

async fn fetch_one(
    client: Arc<Client>,
    account_id: String,
    message_id: String,
    data_dir: PathBuf,
    maildir_name: String,
    flags: Flags,
) -> Result<()> {
    let bytes = fetch_original_message(&client, &account_id, &message_id)
        .await
        .with_context(|| format!("fetching message {message_id}"))?;
    maildir::write_message(&data_dir, &maildir_name, &message_id, flags, &bytes)?;
    Ok(())
}

fn apply_deletes(data_dir: &std::path::Path, items: &[(String, String)]) -> Result<()> {
    for (folder, mid) in items {
        maildir::delete(data_dir, folder, mid)?;
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
