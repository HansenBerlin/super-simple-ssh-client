use std::time::SystemTime;

use anyhow::Result;
use crate::app::constants::NOT_CONNECTED_MESSAGE;
use crate::app::App;
use crate::model::{
    AuthConfig, AuthKind, ConnectionConfig, HistoryEntry, HistoryState, Mode,
    NewConnectionState, OpenConnection, TryResult,
};
use crate::ssh::connect_ssh;
use crate::storage::save_store;

impl App {
    pub(crate) fn history_range(&self, history_len: usize, has_error: bool) -> (usize, usize) {
        let page_size = self.history_page_size(has_error);
        if history_len == 0 {
            return (0, 0);
        }
        let max_page = (history_len - 1) / page_size;
        let page = self.history_page.min(max_page);
        let start = page * page_size;
        let end = (start + page_size).min(history_len);
        (start, end)
    }

    pub(crate) fn max_history_page(&self, history_len: usize, has_error: bool) -> usize {
        let page_size = self.history_page_size(has_error);
        if history_len == 0 {
            return 0;
        }
        (history_len - 1) / page_size
    }

    pub(crate) fn selected_connected_connection(&self) -> Option<ConnectionConfig> {
        let conn = self.connections.get(self.selected_saved)?;
        if self
            .open_connections
            .iter()
            .any(|candidate| crate::model::same_identity(&candidate.config, conn))
        {
            Some(conn.clone())
        } else {
            None
        }
    }

    pub(crate) fn connect_selected(&mut self) -> Option<ConnectionConfig> {
        if let Some(config) = self.connections.get(self.selected_saved).cloned() {
            if let Err(err) = self.connect_and_open(config.clone()) {
                self.record_connect_error(&config, &err);
                self.set_status(format!("Connection failed: {err}"));
                None
            } else {
                Some(config)
            }
        } else {
            self.set_status("No saved connection selected");
            None
        }
    }

    pub(crate) fn disconnect_selected(&mut self) {
        let Some(config) = self.connections.get(self.selected_saved) else {
            self.set_status("No saved connection selected");
            return;
        };
        if let Some(index) = self
            .open_connections
            .iter()
            .position(|conn| crate::model::same_identity(&conn.config, config))
        {
            self.open_connections.remove(index);
            if self.selected_tab > 0 && self.selected_tab >= index {
                self.selected_tab = self.selected_tab.saturating_sub(1);
            }
            self.set_status("Disconnected");
        } else {
            self.set_status(NOT_CONNECTED_MESSAGE);
        }
    }

    pub(crate) fn sort_connections_by_recent(&mut self, selected_key: Option<String>) {
        self.connections.sort_by(|left, right| {
            let left_ts = left.history.iter().map(|h| h.ts).max().unwrap_or(0);
            let right_ts = right.history.iter().map(|h| h.ts).max().unwrap_or(0);
            right_ts.cmp(&left_ts)
        });
        if let Some(key) = selected_key {
            if let Some(index) = self
                .connections
                .iter()
                .position(|conn| crate::model::connection_key(conn) == key)
            {
                self.selected_saved = index;
                return;
            }
        }
        if self.selected_saved >= self.connections.len() {
            self.selected_saved = self.connections.len().saturating_sub(1);
        }
    }

    pub(crate) fn prefill_new_connection(&self, config: &ConnectionConfig) -> NewConnectionState {
        let mut state = NewConnectionState::default();
        state.name = config.name.clone();
        state.user = config.user.clone();
        state.host = config.host.clone();
        match &config.auth {
            AuthConfig::Password { password } => {
                state.auth_kind = AuthKind::PasswordOnly;
                state.password = password.clone();
            }
            AuthConfig::PrivateKey { path, password } => {
                state.key_path = path.clone();
                if let Some(pass) = password {
                    state.auth_kind = AuthKind::PrivateKeyWithPassword;
                    state.password = pass.clone();
                } else {
                    state.auth_kind = AuthKind::PrivateKey;
                }
            }
        }
        state
    }

    pub(crate) fn run_test_connection(&mut self) {
        match self.build_connection_config() {
            Ok(config) => {
                self.try_result = Some(match self.try_connect(&config) {
                    Ok(()) => TryResult {
                        success: true,
                        message: "Connection OK (not saved)".to_string(),
                    },
                    Err(err) => TryResult {
                        success: false,
                        message: format!("Connection failed: {err}"),
                    },
                });
            }
            Err(err) => {
                self.try_result = Some(TryResult {
                    success: false,
                    message: format!("Missing fields: {err}"),
                });
            }
        }
    }

    pub(crate) fn run_save_connection(&mut self) {
        match self.build_connection_config() {
            Ok(config) => {
                let snapshot = config.clone();
                match self.save_or_connect(config) {
                    Ok(()) => {
                        self.mode = Mode::Normal;
                        self.edit_index = None;
                        self.new_connection_feedback = None;
                    }
                    Err(err) => {
                        self.record_connect_error(&snapshot, &err);
                        self.new_connection_feedback = Some(format!("Connection failed: {err}"));
                    }
                }
            }
            Err(err) => {
                self.new_connection_feedback = Some(format!("Missing fields: {err}"));
            }
        }
    }

    pub(crate) fn record_connect_error(&mut self, config: &ConnectionConfig, err: &anyhow::Error) {
        self.last_error
            .insert(crate::model::connection_key(config), format!("{err}"));
        let mut should_save = false;
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|conn| crate::model::same_identity(conn, config))
        {
            existing.history.push(HistoryEntry {
                ts: crate::model::now_epoch(),
                state: HistoryState::Failure,
            });
            should_save = true;
        }
        if should_save {
            if let Err(err) = self.save_store() {
                self.set_status(format!("Failed to save history: {err}"));
            }
            self.sort_connections_by_recent(Some(crate::model::connection_key(config)));
        }
    }

    pub(crate) fn build_connection_config(&self) -> Result<ConnectionConfig> {
        if self.new_connection.user.trim().is_empty() {
            anyhow::bail!("User is required");
        }
        if self.new_connection.host.trim().is_empty() {
            anyhow::bail!("Host is required");
        }

        let auth = match self.new_connection.auth_kind {
            AuthKind::PasswordOnly => {
                if self.new_connection.password.is_empty() {
                    anyhow::bail!("Password is required");
                }
                AuthConfig::Password {
                    password: self.new_connection.password.clone(),
                }
            }
            AuthKind::PrivateKey => {
                if self.new_connection.key_path.trim().is_empty() {
                    anyhow::bail!("Private key path is required");
                }
                AuthConfig::PrivateKey {
                    path: self.new_connection.key_path.clone(),
                    password: None,
                }
            }
            AuthKind::PrivateKeyWithPassword => {
                if self.new_connection.key_path.trim().is_empty() {
                    anyhow::bail!("Private key path is required");
                }
                if self.new_connection.password.is_empty() {
                    anyhow::bail!("Key password is required");
                }
                AuthConfig::PrivateKey {
                    path: self.new_connection.key_path.clone(),
                    password: Some(self.new_connection.password.clone()),
                }
            }
        };

        Ok(ConnectionConfig {
            name: self.new_connection.name.trim().to_string(),
            user: self.new_connection.user.trim().to_string(),
            host: self.new_connection.host.trim().to_string(),
            auth,
            history: vec![],
            last_remote_dir: None,
        })
    }

    fn connect_and_open(&mut self, mut config: ConnectionConfig) -> Result<()> {
        let session = connect_ssh(&config)?;
        config.history.push(HistoryEntry {
            ts: crate::model::now_epoch(),
            state: HistoryState::Success,
        });
        self.open_connections.push(OpenConnection {
            config: config.clone(),
            session,
            connected_at: SystemTime::now(),
        });
        self.selected_tab = self.open_connections.len().saturating_sub(1);
        self.upsert_connection(config.clone());
        self.save_store()?;
        self.sort_connections_by_recent(Some(crate::model::connection_key(&config)));
        self.last_error
            .remove(&crate::model::connection_key(&config));
        self.set_status(format!("Connected to {}", config.label()));
        Ok(())
    }

    fn save_or_connect(&mut self, mut config: ConnectionConfig) -> Result<()> {
        if let Some(index) = self.edit_index {
            if let Some(existing) = self.connections.get(index) {
                config.history = existing.history.clone();
                config.last_remote_dir = existing.last_remote_dir.clone();
            }
            self.connections.remove(index);
            self.upsert_connection(config);
            self.save_store()?;
            self.set_status("Connection updated");
            return Ok(());
        }
        self.connect_and_open(config)
    }

    fn upsert_connection(&mut self, connection: ConnectionConfig) {
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|c| crate::model::same_identity(c, &connection))
        {
            *existing = connection;
            return;
        }
        self.connections.push(connection);
    }

    pub(super) fn save_store(&self) -> Result<()> {
        let stored = crate::model::StoreFile {
            master: self.master.clone(),
            connections: self
                .connections
                .iter()
                .map(|conn| crate::storage::encrypt_connection(conn, &self.master_key))
                .collect::<Result<Vec<_>>>()?,
            last_local_dir: self
                .last_local_dir
                .as_ref()
                .map(|value| value.to_string_lossy().into_owned()),
        };
        save_store(&self.config_path, &stored)
    }

    pub(crate) fn update_last_remote_dir(&mut self, dir: String) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            return Ok(());
        };
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|candidate| crate::model::same_identity(candidate, &conn))
        {
            existing.last_remote_dir = Some(dir);
            self.save_store()?;
        }
        Ok(())
    }

    pub(crate) fn history_page_size(&self, has_error: bool) -> usize {
        let inner_height = self.details_height.saturating_sub(2) as usize;
        let base_lines = 4 + usize::from(has_error);
        let pre_history = base_lines + 2;
        inner_height.saturating_sub(pre_history).max(1)
    }

    fn try_connect(&self, config: &ConnectionConfig) -> Result<()> {
        let _session = connect_ssh(config)?;
        Ok(())
    }
}
