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
    for entry in
        std::fs::read_dir(data_dir).with_context(|| format!("reading {}", data_dir.display()))?
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
    let cur = folder.join("cur").join(format!(
        "{message_id}{}2,{}",
        info_suffix_separator(),
        flags.encode()
    ));

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

pub fn set_flags(
    data_dir: &Path,
    maildir_name: &str,
    message_id: &str,
    flags: Flags,
) -> Result<()> {
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
            let mut iter =
                std::fs::read_dir(&p).with_context(|| format!("reading {}", p.display()))?;
            if iter.next().is_some() {
                return Ok(false);
            }
        }
    }
    for e in std::fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))? {
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
            std::fs::remove_dir(&p).with_context(|| format!("rmdir {}", p.display()))?;
        }
    }
    std::fs::remove_dir(&folder).with_context(|| format!("rmdir {}", folder.display()))?;
    Ok(true)
}

pub fn list_local_folders(data_dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !data_dir.exists() {
        return Ok(out);
    }
    for entry in
        std::fs::read_dir(data_dir).with_context(|| format!("reading {}", data_dir.display()))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "zoho-mail-sync-test-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn flags_encode_canonically_and_decode_maildir_flags() {
        assert_eq!(Flags::default().encode(), "");
        assert_eq!(
            Flags {
                seen: true,
                flagged: true,
            }
            .encode(),
            "FS"
        );
        assert_eq!(
            Flags::decode("RST"),
            Flags {
                seen: true,
                flagged: false,
            }
        );
        assert_eq!(
            Flags::decode("DF"),
            Flags {
                seen: false,
                flagged: true,
            }
        );
    }

    #[test]
    fn message_lifecycle_is_visible_to_scans() {
        let root = TestDir::new();
        ensure_folder(&root.0, ".Inbox").unwrap();
        ensure_folder(&root.0, ".Archive").unwrap();

        write_message(
            &root.0,
            ".Inbox",
            "message-1",
            Flags {
                seen: true,
                flagged: false,
            },
            b"Subject: test\r\n\r\nbody",
        )
        .unwrap();

        let local = scan_data_dir(&root.0).unwrap();
        let message = local.get("message-1").unwrap();
        assert_eq!(message.maildir_name, ".Inbox");
        assert_eq!(
            message.flags,
            Flags {
                seen: true,
                flagged: false,
            }
        );

        move_to_folder(
            &root.0,
            ".Inbox",
            ".Archive",
            "message-1",
            Flags {
                seen: true,
                flagged: true,
            },
        )
        .unwrap();

        let local = scan_data_dir(&root.0).unwrap();
        let message = local.get("message-1").unwrap();
        assert_eq!(message.maildir_name, ".Archive");
        assert_eq!(
            message.flags,
            Flags {
                seen: true,
                flagged: true,
            }
        );

        delete(&root.0, ".Archive", "message-1").unwrap();
        assert!(scan_data_dir(&root.0).unwrap().is_empty());
    }

    #[test]
    fn local_folder_listing_excludes_state_and_non_maildir_entries() {
        let root = TestDir::new();
        ensure_folder(&root.0, ".Inbox").unwrap();
        ensure_folder(&root.0, ".Sent").unwrap();
        std::fs::create_dir(root.0.join(".zoho-mail-sync")).unwrap();
        std::fs::create_dir(root.0.join("visible-directory")).unwrap();
        std::fs::write(root.0.join(".hidden-file"), b"not a directory").unwrap();

        let mut folders = list_local_folders(&root.0).unwrap();
        folders.sort();
        assert_eq!(folders, vec![".Inbox", ".Sent"]);
    }

    #[test]
    fn stale_folder_removal_requires_an_empty_maildir() {
        let root = TestDir::new();
        ensure_folder(&root.0, ".Empty").unwrap();
        assert!(rmdir_if_empty(&root.0, ".Empty").unwrap());
        assert!(!root.0.join(".Empty").exists());

        ensure_folder(&root.0, ".NonEmpty").unwrap();
        write_message(
            &root.0,
            ".NonEmpty",
            "message-2",
            Flags::default(),
            b"Subject: retained\r\n\r\nbody",
        )
        .unwrap();
        assert!(!rmdir_if_empty(&root.0, ".NonEmpty").unwrap());
        assert!(root.0.join(".NonEmpty").exists());
    }
}
