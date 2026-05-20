use anyhow::{Context, Result};
use maildir::Maildir;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
}

impl Flags {
    pub fn encode(&self) -> String {
        let mut s = String::new();
        if self.flagged {
            s.push('F');
        }
        if self.seen {
            s.push('S');
        }
        s
    }

    pub fn decode(s: &str) -> Self {
        Flags {
            seen: s.contains('S'),
            flagged: s.contains('F'),
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
        let md = Maildir::from(folder_path.clone());
        for mail in md.list_cur().chain(md.list_new()) {
            match mail {
                Ok(m) => {
                    out.insert(
                        m.id().to_string(),
                        LocalEntry {
                            maildir_name: name_str.to_string(),
                            flags: Flags::decode(m.flags()),
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(folder = %folder_path.display(), "skipping unparseable maildir entry: {e}");
                }
            }
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
    let tmp = folder.join("tmp").join(message_id);
    let cur = folder
        .join("cur")
        .join(format!("{message_id}{}2,{}", info_suffix_separator(), flags.encode()));

    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .with_context(|| format!("opening {}", tmp.display()))?;
        f.write_all(body)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("fsync {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &cur)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), cur.display()))?;
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

pub fn rmdir_if_empty(data_dir: &Path, maildir_name: &str) -> Result<bool> {
    let folder = data_dir.join(maildir_name);
    if !folder.is_dir() {
        return Ok(false);
    }
    for sub in ["cur", "new", "tmp"] {
        let p = folder.join(sub);
        if p.is_dir() {
            let mut iter = std::fs::read_dir(&p)
                .with_context(|| format!("reading {}", p.display()))?;
            if iter.next().is_some() {
                return Ok(false);
            }
        }
    }
    for e in std::fs::read_dir(&folder)
        .with_context(|| format!("reading {}", folder.display()))?
    {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if n != "cur" && n != "new" && n != "tmp" {
            return Ok(false);
        }
    }
    for sub in ["cur", "new", "tmp"] {
        let p = folder.join(sub);
        if p.is_dir() {
            std::fs::remove_dir(&p)
                .with_context(|| format!("rmdir {}", p.display()))?;
        }
    }
    std::fs::remove_dir(&folder)
        .with_context(|| format!("rmdir {}", folder.display()))?;
    Ok(true)
}

pub fn list_local_folders(data_dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !data_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(data_dir)
        .with_context(|| format!("reading {}", data_dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !name_str.starts_with('.') || name_str == "." || name_str == ".." {
            continue;
        }
        if name_str == ".zoho-mail-sync" {
            continue;
        }
        if !entry.path().is_dir() {
            continue;
        }
        out.push(name_str);
    }
    Ok(out)
}
