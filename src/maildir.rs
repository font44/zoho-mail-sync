use anyhow::{Context, Result};
use maildir::Maildir;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub draft: bool,
    pub trashed: bool,
}

impl Flags {
    pub fn encode(&self) -> String {
        let mut s = String::new();
        if self.draft {
            s.push('D');
        }
        if self.flagged {
            s.push('F');
        }
        if self.seen {
            s.push('S');
        }
        if self.trashed {
            s.push('T');
        }
        s
    }

    pub fn decode(s: &str) -> Self {
        Flags {
            seen: s.contains('S'),
            flagged: s.contains('F'),
            draft: s.contains('D'),
            trashed: s.contains('T'),
        }
    }
}

pub fn ensure_folder(data_dir: &Path, maildir_name: &str) -> Result<PathBuf> {
    let folder = data_dir.join(maildir_name);
    let md = Maildir::from(folder.clone());
    md.create_dirs()
        .with_context(|| format!("creating maildir at {}", folder.display()))?;
    Ok(folder)
}

#[derive(Debug, Clone)]
pub struct LocalEntry {
    pub maildir_name: String,
    pub flags: Flags,
}

pub fn scan_data_dir(data_dir: &Path) -> Result<HashMap<String, LocalEntry>> {
    let mut out = HashMap::new();
    if !data_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(data_dir)
        .with_context(|| format!("reading {}", data_dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        if !name_str.starts_with('.') || name_str == "." || name_str == ".." {
            continue;
        }
        if name_str == ".zoho-mail-sync" {
            continue;
        }
        let folder_path = entry.path();
        if !folder_path.is_dir() {
            continue;
        }
        let md = Maildir::from(folder_path);
        for mail in md.list_cur().chain(md.list_new()) {
            let mail = match mail {
                Ok(m) => m,
                Err(_) => continue,
            };
            out.insert(
                mail.id().to_string(),
                LocalEntry {
                    maildir_name: name_str.to_string(),
                    flags: Flags::decode(mail.flags()),
                },
            );
        }
    }
    Ok(out)
}

pub fn write_message(
    data_dir: &Path,
    maildir_name: &str,
    message_id: &str,
    flags: Flags,
    body: &[u8],
) -> Result<()> {
    let folder = data_dir.join(maildir_name);
    let md = Maildir::from(folder.clone());
    let auto_id = md
        .store_cur_with_flags(body, &flags.encode())
        .map_err(|e| anyhow::anyhow!("storing message {message_id}: {e}"))?;
    let auto_entry = md
        .find(&auto_id)
        .ok_or_else(|| anyhow::anyhow!("just-stored message {auto_id} disappeared"))?;
    let parent = auto_entry
        .path()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent for stored message"))?
        .to_path_buf();
    let separator = info_suffix_separator();
    let target = parent.join(format!("{message_id}{separator}2,{}", flags.encode()));
    std::fs::rename(auto_entry.path(), &target)
        .with_context(|| format!("renaming {} -> {}", auto_entry.path().display(), target.display()))?;
    Ok(())
}

#[cfg(unix)]
fn info_suffix_separator() -> &'static str {
    ":"
}

#[cfg(windows)]
fn info_suffix_separator() -> &'static str {
    ";"
}

pub fn set_flags(data_dir: &Path, maildir_name: &str, message_id: &str, flags: Flags) -> Result<()> {
    let folder = data_dir.join(maildir_name);
    let md = Maildir::from(folder);
    md.set_flags(message_id, &flags.encode())
        .with_context(|| format!("setting flags on {message_id} in {maildir_name}"))
}

pub fn move_to_folder(
    data_dir: &Path,
    from_maildir: &str,
    to_maildir: &str,
    message_id: &str,
    flags: Flags,
) -> Result<()> {
    let src = Maildir::from(data_dir.join(from_maildir));
    let dst = Maildir::from(data_dir.join(to_maildir));
    src.move_to(message_id, &dst)
        .with_context(|| format!("moving {message_id} from {from_maildir} to {to_maildir}"))?;
    set_flags(data_dir, to_maildir, message_id, flags)
}

pub fn delete(data_dir: &Path, maildir_name: &str, message_id: &str) -> Result<()> {
    let folder = data_dir.join(maildir_name);
    let md = Maildir::from(folder);
    md.delete(message_id)
        .with_context(|| format!("deleting {message_id} from {maildir_name}"))
}
