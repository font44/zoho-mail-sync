use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use super::Client;

const PAGE_LIMIT: u32 = 200;

#[derive(Debug, Deserialize)]
struct MessagesEnvelope {
    #[serde(default)]
    data: Vec<MessageEntry>,
}

#[derive(Debug, Deserialize)]
struct MessageEntry {
    #[serde(rename = "messageId")]
    message_id: serde_json::Value,
    #[serde(rename = "folderId")]
    folder_id: serde_json::Value,
    #[serde(default)]
    status: Option<serde_json::Value>,
    #[serde(default)]
    flagid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteMessage {
    pub message_id: String,
    pub folder_id: String,
    pub read: bool,
    pub important: bool,
}

pub async fn list_folder_messages(
    client: &Client,
    account_id: &str,
    folder_id: &str,
) -> Result<Vec<RemoteMessage>> {
    let mut out = Vec::new();
    let mut start: u32 = 1;
    loop {
        let url = format!(
            "{}/accounts/{account_id}/messages/view?folderId={folder_id}&start={start}&limit={PAGE_LIMIT}&sortBy=date&sortorder=false",
            client.api_base()
        );
        let env: MessagesEnvelope = client
            .get_json(&url)
            .await
            .with_context(|| format!("listing messages from {url}"))?;
        let len = env.data.len();
        for m in env.data {
            let message_id = match value_to_string(&m.message_id) {
                Some(s) => s,
                None => continue,
            };
            let folder_id = match value_to_string(&m.folder_id) {
                Some(s) => s,
                None => continue,
            };
            let read = m
                .status
                .as_ref()
                .and_then(value_to_string)
                .map(|s| s == "1")
                .unwrap_or(false);
            let important = m
                .flagid
                .as_deref()
                .map(|s| s == "important")
                .unwrap_or(false);
            out.push(RemoteMessage {
                message_id,
                folder_id,
                read,
                important,
            });
        }
        if len < PAGE_LIMIT as usize {
            break;
        }
        start = start
            .checked_add(PAGE_LIMIT)
            .ok_or_else(|| anyhow!("paging overflow"))?;
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct OriginalMessageEnvelope {
    data: OriginalMessageData,
}

#[derive(Debug, Deserialize)]
struct OriginalMessageData {
    content: String,
}

pub async fn fetch_original_message(
    client: &Client,
    account_id: &str,
    message_id: &str,
) -> Result<Vec<u8>> {
    let url = format!(
        "{}/accounts/{account_id}/messages/{message_id}/originalmessage",
        client.api_base()
    );
    let env: OriginalMessageEnvelope = client
        .get_json(&url)
        .await
        .with_context(|| format!("fetching original message from {url}"))?;
    Ok(env.data.content.into_bytes())
}

fn value_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

