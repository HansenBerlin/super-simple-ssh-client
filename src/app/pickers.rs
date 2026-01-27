use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;

use crate::app::helpers::{read_dir_entries_filtered, resolve_picker_start};
use crate::app::App;
use crate::model::{AuthConfig, FilePickerState, KeyPickerState, RemoteEntry, RemotePickerState};

impl App {
    pub(crate) fn open_file_picker(&mut self) -> Result<()> {
        let start_dir = resolve_picker_start(&self.new_connection.key_path)?;
        let entries = read_dir_entries_filtered(&start_dir, false, false)?;
        self.file_picker = Some(FilePickerState {
            cwd: start_dir,
            entries,
            selected: 0,
            show_hidden: false,
        });
        Ok(())
    }

    pub(crate) fn open_local_picker(&mut self, start: Option<PathBuf>, only_dirs: bool) -> Result<()> {
        let start_dir = match start {
            Some(dir) => dir,
            None => self
                .last_local_dir
                .clone()
                .filter(|dir| dir.is_dir())
                .unwrap_or_else(|| resolve_picker_start("").unwrap_or_else(|_| PathBuf::from("."))),
        };
        let entries = read_dir_entries_filtered(&start_dir, only_dirs, false)?;
        self.file_picker = Some(FilePickerState {
            cwd: start_dir,
            entries,
            selected: 0,
            show_hidden: false,
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
            self.connections
                .iter()
                .find(|candidate| crate::model::same_identity(candidate, &conn))
                .and_then(|candidate| candidate.last_remote_dir.clone())
                .unwrap_or_else(|| format!("/home/{}", conn.user))
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
            show_hidden: false,
        });
        if self.try_load_remote_dir(cwd.clone(), only_dirs).is_ok() {
            return Ok(());
        }
        if let Some(next) = self.remote_home_fallback() {
            if self.try_load_remote_dir(next.clone(), only_dirs).is_ok() {
                if let Some(picker) = &mut self.remote_picker {
                    picker.cwd = next;
                }
                return Ok(());
            }
        }
        if let Err(err) = self.start_remote_fetch("/".to_string(), only_dirs) {
            return Err(err);
        }
        if let Some(picker) = &mut self.remote_picker {
            picker.cwd = "/".to_string();
        }
        Ok(())
    }

    pub(crate) fn load_remote_dir(&mut self, cwd: String, only_dirs: bool) -> Result<()> {
        if let Some(picker) = &mut self.remote_picker {
            picker.cwd = cwd.clone();
            picker.entries.clear();
            picker.selected = 0;
            picker.loading = true;
            picker.error = None;
        }
        if self.try_load_remote_dir(cwd.clone(), only_dirs).is_ok() {
            return Ok(());
        }
        if let Some(next) = self.remote_home_fallback() {
            if self.try_load_remote_dir(next.clone(), only_dirs).is_ok() {
                if let Some(picker) = &mut self.remote_picker {
                    picker.cwd = next;
                }
                return Ok(());
            }
        }
        if let Err(err) = self.start_remote_fetch("/".to_string(), only_dirs) {
            return Err(err);
        }
        if let Some(picker) = &mut self.remote_picker {
            picker.cwd = "/".to_string();
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
        let show_hidden = self
            .remote_picker
            .as_ref()
            .map(|picker| picker.show_hidden)
            .unwrap_or(false);
        let (tx, rx) = mpsc::channel();
        let backend = self.ssh_backend.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<RemoteEntry>> {
                backend.list_remote_dir(None, &conn, &cwd, only_dirs, show_hidden)
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

    fn try_load_remote_dir(&mut self, cwd: String, only_dirs: bool) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            anyhow::bail!("Selected connection is not connected");
        };
        let show_hidden = self
            .remote_picker
            .as_ref()
            .map(|picker| picker.show_hidden)
            .unwrap_or(false);
        let entries = self
            .ssh_backend
            .list_remote_dir(
                Some(&self.open_connections),
                &conn,
                &cwd,
                only_dirs,
                show_hidden,
            )?;
        if let Some(picker) = &mut self.remote_picker {
            picker.entries = entries;
            picker.loading = false;
            picker.error = None;
        }
        Ok(())
    }

    fn remote_home_fallback(&self) -> Option<String> {
        let conn = self.selected_connected_connection()?;
        let home = self
            .ssh_backend
            .remote_home_dir(Some(&self.open_connections), &conn)
            .ok()
            .flatten()?;
        let trimmed = home.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ssh_backend::MockSshBackend;
    use crate::model::{AuthConfig, ConnectionConfig, OpenConnection, RemoteEntry};
    use std::sync::Arc;
    use std::time::SystemTime;

    #[test]
    fn open_local_picker_uses_last_dir() {
        let mut app = App::for_test();
        let temp = std::env::temp_dir();
        app.last_local_dir = Some(temp.clone());
        app.open_local_picker(None, false).unwrap();
        let picker = app.file_picker.as_ref().unwrap();
        assert_eq!(picker.cwd, temp);
    }

    #[test]
    fn open_remote_picker_falls_back_to_home() {
        let backend = Arc::new(MockSshBackend::default());
        backend.set_list("/home/root", Err(anyhow::anyhow!("missing")));
        backend.set_list(
            "/home/user",
            Ok(vec![RemoteEntry {
                name: "etc".to_string(),
                path: "/home/user/etc".to_string(),
                is_dir: true,
            }]),
        );
        backend.set_home(Some("/home/user".to_string()));

        let mut app = App::for_test_with_backend(backend);
        let connection = ConnectionConfig {
            name: "test".to_string(),
            user: "root".to_string(),
            host: "host".to_string(),
            auth: AuthConfig::Password {
                password: "pw".to_string(),
            },
            history: vec![],
            last_remote_dir: None,
        };
        app.connections.push(connection.clone());
        app.open_connections.push(OpenConnection {
            config: connection,
            session: ssh2::Session::new().unwrap(),
            connected_at: SystemTime::now(),
        });
        app.selected_saved = 0;
        app.open_remote_picker_at("/home/root".to_string(), false).unwrap();
        app.poll_remote_fetch();
        let picker = app.remote_picker.as_ref().unwrap();
        assert_eq!(picker.cwd, "/home/user");
    }
}
