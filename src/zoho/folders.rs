use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;

use super::Client;

#[derive(Debug, Deserialize)]
struct AccountsEnvelope {
    data: Vec<AccountEntry>,
}

#[derive(Debug, Deserialize)]
struct AccountEntry {
    #[serde(rename = "accountId")]
    account_id: String,
}

pub async fn list_account_ids(client: &Client) -> Result<Vec<String>> {
    let url = format!("{}/accounts", client.api_base());
    let env: AccountsEnvelope = client.get_json(&url).await?;
    Ok(env.data.into_iter().map(|e| e.account_id).collect())
}

pub async fn discover_account_id(client: &Client) -> Result<String> {
    let url = format!("{}/accounts", client.api_base());
    list_account_ids(client)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no accounts returned by {url}"))
}

#[derive(Debug, Deserialize)]
struct FoldersEnvelope {
    data: Vec<FolderEntry>,
}

#[derive(Debug, Deserialize)]
struct FolderEntry {
    #[serde(rename = "folderId")]
    folder_id: String,
    #[serde(rename = "folderName")]
    folder_name: String,
    #[serde(default, rename = "parentFolderId")]
    parent_folder_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub folder_id: String,
    pub maildir_name: String,
}

pub async fn list_folders(client: &Client, account_id: &str) -> Result<Vec<Folder>> {
    let url = format!("{}/accounts/{account_id}/folders", client.api_base());
    let env: FoldersEnvelope = client
        .get_json(&url)
        .await
        .with_context(|| format!("listing folders from {url}"))?;
    let raw: Vec<(String, String, Option<String>)> = env
        .data
        .into_iter()
        .map(|f| {
            let parent = f.parent_folder_id.filter(|s| !s.is_empty() && s != "0");
            (f.folder_id, f.folder_name, parent)
        })
        .collect();
    Ok(derive_maildir_names(raw))
}

fn derive_maildir_names(raw: Vec<(String, String, Option<String>)>) -> Vec<Folder> {
    let by_id: HashMap<String, (String, Option<String>)> = raw
        .iter()
        .map(|(fid, name, parent)| (fid.clone(), (name.clone(), parent.clone())))
        .collect();

    raw.iter()
        .map(|(fid, name, parent)| {
            let mut chain: Vec<String> = vec![sanitize(name)];
            let mut cur_parent = parent.clone();
            let mut depth = 0;
            while let Some(pid) = cur_parent {
                if depth > 32 {
                    break;
                }
                if let Some((pname, pparent)) = by_id.get(&pid) {
                    chain.push(sanitize(pname));
                    cur_parent = pparent.clone();
                } else {
                    break;
                }
                depth += 1;
            }
            chain.reverse();
            let maildir_name = format!(".{}", chain.join("."));
            Folder {
                folder_id: fid.clone(),
                maildir_name,
            }
        })
        .collect()
}

fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '/' | '.' | '\\' | ':' => out.push('_'),
            c if c.is_control() => out.push('_'),
            c => out.push(c),
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}
