use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use super::Client;

#[derive(Debug, Deserialize)]
struct MessagesEnvelope {
    #[serde(default)]
    data: Vec<MessageEntry>,
}

#[derive(Debug, Deserialize)]
struct MessageEntry {
    #[serde(rename = "messageId")]
    message_id: String,
    #[serde(rename = "folderId")]
    folder_id: String,
    #[serde(default)]
    status: Option<String>,
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
    let page_limit = client.page_limit().max(1);
    loop {
        let url = format!(
            "{}/accounts/{account_id}/messages/view?folderId={folder_id}&start={start}&limit={page_limit}&sortBy=date&sortorder=false",
            client.api_base()
        );
        let env: MessagesEnvelope = client
            .get_json(&url)
            .await
            .with_context(|| format!("listing messages from {url}"))?;
        let len = env.data.len();
        for m in env.data {
            let read = m.status.as_deref() == Some("1");
            let important = m.flagid.as_deref() == Some("important");
            out.push(RemoteMessage {
                message_id: m.message_id,
                folder_id: m.folder_id,
                read,
                important,
            });
        }
        if len < page_limit as usize {
            break;
        }
        start = start
            .checked_add(page_limit)
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
