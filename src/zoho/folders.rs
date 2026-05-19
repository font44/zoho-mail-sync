use anyhow::{anyhow, Context, Result};
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
    account_id: serde_json::Value,
}

pub async fn discover_account_id(client: &Client) -> Result<String> {
    let url = format!("{}/accounts", client.api_base());
    let env: AccountsEnvelope = client.get_json(&url).await?;
    let entry = env
        .data
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no accounts returned by {url}"))?;
    json_value_to_string(&entry.account_id)
        .ok_or_else(|| anyhow!("accountId was neither string nor number"))
}

#[derive(Debug, Deserialize)]
struct FoldersEnvelope {
    data: Vec<FolderEntry>,
}

#[derive(Debug, Deserialize)]
struct FolderEntry {
    #[serde(rename = "folderId")]
    folder_id: serde_json::Value,
    #[serde(rename = "folderName")]
    folder_name: String,
    #[serde(default, rename = "parentFolderId")]
    parent_folder_id: Option<serde_json::Value>,
    #[serde(default, rename = "folderType")]
    folder_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub folder_id: String,
    pub folder_type: Option<String>,
    pub maildir_name: String,
}

pub async fn list_folders(client: &Client, account_id: &str) -> Result<Vec<Folder>> {
    let url = format!("{}/accounts/{account_id}/folders", client.api_base());
    let env: FoldersEnvelope = client
        .get_json(&url)
        .await
        .with_context(|| format!("listing folders from {url}"))?;
    let raw: Vec<(String, String, Option<String>, Option<String>)> = env
        .data
        .into_iter()
        .filter_map(|f| {
            let fid = json_value_to_string(&f.folder_id)?;
            let parent = f.parent_folder_id.as_ref().and_then(json_value_to_string);
            let parent = parent.filter(|s| !s.is_empty() && s != "0");
            Some((fid, f.folder_name, parent, f.folder_type))
        })
        .collect();
    Ok(derive_maildir_names(raw))
}

fn json_value_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn derive_maildir_names(
    raw: Vec<(String, String, Option<String>, Option<String>)>,
) -> Vec<Folder> {
    let by_id: HashMap<String, (String, Option<String>, Option<String>)> = raw
        .iter()
        .map(|(fid, name, parent, ftype)| (fid.clone(), (name.clone(), parent.clone(), ftype.clone())))
        .collect();

    raw.iter()
        .map(|(fid, name, parent, ftype)| {
            let mut chain: Vec<String> = vec![sanitize(name)];
            let mut cur_parent = parent.clone();
            let mut depth = 0;
            while let Some(pid) = cur_parent {
                if depth > 32 {
                    break;
                }
                if let Some((pname, pparent, _)) = by_id.get(&pid) {
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
                folder_type: ftype.clone(),
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

