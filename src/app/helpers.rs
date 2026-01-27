use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::FileEntry;
use crate::ssh::expand_tilde;

pub(crate) fn resolve_picker_start(current: &str) -> Result<PathBuf> {
    if !current.trim().is_empty() {
        let path = expand_tilde(current);
        if path.is_dir() {
            return Ok(path);
        }
        if let Some(parent) = path.parent() {
            return Ok(parent.to_path_buf());
        }
    }
    if let Some(home) = dirs::home_dir() {
        return Ok(home);
    }
    std::env::current_dir().context("current dir")
}

pub(crate) fn read_dir_entries_filtered(dir: &Path, only_dirs: bool) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).context("read dir")? {
        let entry = entry.context("read dir entry")?;
        let path = entry.path();
        let file_type = entry.file_type().context("read file type")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if only_dirs && !file_type.is_dir() {
            continue;
        }
        entries.push(FileEntry {
            name,
            path,
            is_dir: file_type.is_dir(),
        });
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
}

pub(crate) fn compute_local_size(path: &Option<PathBuf>, is_dir: bool) -> Result<u64> {
    let Some(path) = path else {
        anyhow::bail!("missing source");
    };
    if !is_dir {
        let meta = fs::metadata(path).context("stat file")?;
        return Ok(meta.len());
    }
    fn walk(dir: &Path) -> Result<u64> {
        let mut total = 0u64;
        for entry in fs::read_dir(dir).context("read dir")? {
            let entry = entry.context("read dir entry")?;
            let path = entry.path();
            let meta = entry.metadata().context("stat entry")?;
            if meta.is_dir() {
                total = total.saturating_add(walk(&path)?);
            } else {
                total = total.saturating_add(meta.len());
            }
        }
        Ok(total)
    }
    walk(path)
}

pub(crate) fn parent_remote_dir(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(base, _)| if base.is_empty() { "/".to_string() } else { base.to_string() })
        .unwrap_or_else(|| "/".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn temp_dir() -> PathBuf {
        let mut base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        base.push(format!("ssh-client-test-{nanos}"));
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn read_dir_entries_filtered_sorts_and_filters() {
        let root = temp_dir();
        let file_path = root.join("b.txt");
        let dir_path = root.join("a-dir");
        fs::create_dir_all(&dir_path).unwrap();
        fs::write(&file_path, b"x").unwrap();

        let entries = read_dir_entries_filtered(&root, false).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a-dir");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "b.txt");
        assert!(!entries[1].is_dir);

        let dirs_only = read_dir_entries_filtered(&root, true).unwrap();
        assert_eq!(dirs_only.len(), 1);
        assert_eq!(dirs_only[0].name, "a-dir");
        assert!(dirs_only[0].is_dir);
    }

    #[test]
    fn compute_local_size_file_and_dir() {
        let root = temp_dir();
        let file_path = root.join("file.bin");
        fs::write(&file_path, vec![0u8; 5]).unwrap();

        let size = compute_local_size(&Some(file_path.clone()), false).unwrap();
        assert_eq!(size, 5);

        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let mut f = fs::File::create(nested.join("a.txt")).unwrap();
        f.write_all(b"hello").unwrap();
        fs::write(nested.join("b.txt"), b"abc").unwrap();

        let size = compute_local_size(&Some(root.clone()), true).unwrap();
        assert_eq!(size, 5 + 5 + 3);
    }

    #[test]
    fn parent_remote_dir_handles_root_and_trailing() {
        assert_eq!(parent_remote_dir("/"), "/");
        assert_eq!(parent_remote_dir("/home/user/"), "/home");
        assert_eq!(parent_remote_dir("/home/user"), "/home");
        assert_eq!(parent_remote_dir("home"), "/");
    }

    #[test]
    fn resolve_picker_start_uses_current_or_home() {
        let path = resolve_picker_start("").unwrap();
        assert!(path.is_dir());
        let temp = std::env::temp_dir();
        let nested = temp.join("picker-test-file.txt");
        fs::write(&nested, b"x").unwrap();
        let resolved = resolve_picker_start(nested.to_string_lossy().as_ref()).unwrap();
        assert!(resolved.is_dir());
    }
}
