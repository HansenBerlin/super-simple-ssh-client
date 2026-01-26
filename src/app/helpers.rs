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
