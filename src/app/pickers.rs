use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context, Result};

use crate::app::helpers::{read_dir_entries_filtered, resolve_picker_start};
use crate::app::App;
use crate::model::{AuthConfig, FilePickerState, KeyPickerState, RemoteEntry, RemotePickerState};
use crate::ssh::connect_ssh;

impl App {
    pub(crate) fn open_file_picker(&mut self) -> Result<()> {
        let start_dir = resolve_picker_start(&self.new_connection.key_path)?;
        let entries = read_dir_entries_filtered(&start_dir, false)?;
        self.file_picker = Some(FilePickerState {
            cwd: start_dir,
            entries,
            selected: 0,
        });
        Ok(())
    }

    pub(crate) fn open_local_picker(&mut self, start: Option<PathBuf>, only_dirs: bool) -> Result<()> {
        let start_dir = match start {
            Some(dir) => dir,
            None => resolve_picker_start("")?,
        };
        let entries = read_dir_entries_filtered(&start_dir, only_dirs)?;
        self.file_picker = Some(FilePickerState {
            cwd: start_dir,
            entries,
            selected: 0,
        });
        Ok(())
    }

    pub(crate) fn open_key_picker(&mut self) {
        let keys = self.collect_key_candidates();
        if keys.is_empty() {
            self.set_status("No known keys yet");
            return;
        }
        self.key_picker = Some(KeyPickerState { keys, selected: 0 });
    }

    pub(crate) fn open_remote_picker(&mut self) -> Result<()> {
        let cwd = if let Some(conn) = self.selected_connected_connection() {
            format!("/home/{}", conn.user)
        } else {
            "/".to_string()
        };
        let only_dirs = self.transfer.as_ref().is_some_and(|t| {
            t.direction == crate::model::TransferDirection::Upload
                && t.step == crate::model::TransferStep::PickTarget
        });
        self.open_remote_picker_at(cwd, only_dirs)
    }

    pub(crate) fn open_remote_picker_at(&mut self, cwd: String, only_dirs: bool) -> Result<()> {
        self.remote_picker = Some(RemotePickerState {
            cwd: cwd.clone(),
            entries: vec![],
            selected: 0,
            loading: true,
            error: None,
            only_dirs,
        });
        if let Err(_err) = self.start_remote_fetch(cwd.clone(), only_dirs) {
            if let Err(err) = self.start_remote_fetch("/".to_string(), only_dirs) {
                return Err(err);
            }
            if let Some(picker) = &mut self.remote_picker {
                picker.cwd = "/".to_string();
            }
        }
        Ok(())
    }

    pub(crate) fn open_local_target_picker(&mut self) -> Result<()> {
        self.open_local_target_picker_at(None)
    }

    pub(crate) fn open_local_target_picker_at(&mut self, start: Option<PathBuf>) -> Result<()> {
        self.open_local_picker(start, true)
    }

    pub(crate) fn start_remote_fetch(&mut self, cwd: String, only_dirs: bool) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            anyhow::bail!("Selected connection is not connected");
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<RemoteEntry>> {
                let session = connect_ssh(&conn)?;
                let sftp = session.sftp().context("open sftp")?;
                let mut entries = Vec::new();
                for (path, stat) in sftp.readdir(Path::new(&cwd)).context("read remote dir")? {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| String::from("/"));
                    if name == "." || name == ".." {
                        continue;
                    }
                    let is_dir = stat.perm.unwrap_or(0) & 0o040000 != 0;
                    if only_dirs && !is_dir {
                        continue;
                    }
                    let full = if cwd == "/" {
                        format!("/{name}")
                    } else {
                        format!("{cwd}/{name}")
                    };
                    entries.push(RemoteEntry {
                        name,
                        path: full,
                        is_dir,
                    });
                }
                entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                Ok(entries)
            })();
            let _ = tx.send(result);
        });
        self.remote_fetch = Some(rx);
        Ok(())
    }

    pub(crate) fn poll_remote_fetch(&mut self) {
        let Some(rx) = &self.remote_fetch else {
            return;
        };
        if let Ok(result) = rx.try_recv() {
            if let Some(picker) = &mut self.remote_picker {
                picker.loading = false;
                match result {
                    Ok(entries) => {
                        if picker.only_dirs {
                            picker.entries = entries.into_iter().filter(|e| e.is_dir).collect();
                        } else {
                            picker.entries = entries;
                        }
                        picker.error = None;
                    }
                    Err(err) => {
                        picker.error = Some(err.to_string());
                    }
                }
            }
            self.remote_fetch = None;
        }
    }

    fn collect_key_candidates(&self) -> Vec<crate::model::KeyCandidate> {
        let mut candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for conn in &self.connections {
            if let AuthConfig::PrivateKey { path, password } = &conn.auth {
                if seen.insert(path.clone()) {
                    candidates.push(crate::model::KeyCandidate {
                        path: path.clone(),
                        password: password.clone(),
                    });
                }
            }
        }
        candidates
    }
}
