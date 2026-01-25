use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as Base64;
use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::model::{
    AppAction, AuthConfig, AuthKind, ConnectionConfig, Field, FileEntry, FilePickerState,
    HistoryEntry, HistoryState, KeyPickerState, MasterField, MasterPasswordState, Mode,
    NewConnectionState, Notice, OpenConnection, RemoteEntry, RemotePickerState, TransferState,
    TransferStep, TryResult,
};
use crate::ssh::{connect_ssh, expand_tilde, run_ssh_terminal};
use crate::storage::{
    config_path, create_master_from_password, load_or_init_store, save_store,
};

pub(crate) struct App {
    pub(crate) config_path: PathBuf,
    pub(crate) master: crate::model::MasterConfig,
    pub(crate) master_key: Vec<u8>,
    pub(crate) connections: Vec<ConnectionConfig>,
    pub(crate) selected_saved: usize,
    pub(crate) selected_tab: usize,
    pub(crate) open_connections: Vec<OpenConnection>,
    pub(crate) mode: Mode,
    pub(crate) new_connection: NewConnectionState,
    pub(crate) master_change: MasterPasswordState,
    pub(crate) status: String,
    pub(crate) file_picker: Option<FilePickerState>,
    pub(crate) key_picker: Option<KeyPickerState>,
    pub(crate) pending_action: Option<AppAction>,
    pub(crate) last_error: HashMap<String, String>,
    pub(crate) edit_index: Option<usize>,
    pub(crate) delete_index: Option<usize>,
    pub(crate) try_result: Option<TryResult>,
    pub(crate) new_connection_feedback: Option<String>,
    pub(crate) notice: Option<Notice>,
    pub(crate) show_help: bool,
    pub(crate) show_header: bool,
    pub(crate) history_page: usize,
    pub(crate) details_height: u16,
    pub(crate) transfer: Option<TransferState>,
    pub(crate) remote_picker: Option<RemotePickerState>,
    remote_fetch: Option<mpsc::Receiver<Result<Vec<RemoteEntry>>>>,
}

impl App {
    pub(crate) fn load_with_master() -> Result<Self> {
        let config_path = config_path()?;
        let (master, master_key, connections) = load_or_init_store(&config_path)?;
        Ok(Self {
            config_path,
            master,
            master_key,
            connections,
            selected_saved: 0,
            selected_tab: 0,
            open_connections: vec![],
            mode: Mode::Normal,
            new_connection: NewConnectionState::default(),
            master_change: MasterPasswordState::default(),
            status: "Ready".to_string(),
            file_picker: None,
            key_picker: None,
            pending_action: None,
            last_error: HashMap::new(),
            edit_index: None,
            delete_index: None,
            try_result: None,
            new_connection_feedback: None,
            notice: None,
            show_help: true,
            show_header: true,
            history_page: 0,
            details_height: 0,
            transfer: None,
            remote_picker: None,
            remote_fetch: None,
        })
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.notice.is_some() {
            if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                self.notice = None;
            }
            return Ok(false);
        }
        self.poll_remote_fetch();
        if self.transfer.is_some() {
            if self.file_picker.is_some() {
                return self.handle_file_picker_key(key);
            }
            if self.remote_picker.is_some() {
                return self.handle_remote_picker_key(key);
            }
            if matches!(self.transfer.as_ref().map(|t| t.step), Some(TransferStep::Confirm)) {
                return self.handle_transfer_confirm(key);
            }
        }
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::NewConnection => self.handle_new_connection_key(key),
            Mode::ChangeMasterPassword => self.handle_master_password_key(key),
            Mode::ConfirmDelete => self.handle_confirm_delete_key(key),
        }
    }

    pub(crate) fn handle_terminal_mode(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            self.status = "Selected connection is not connected".to_string();
            return Ok(());
        };
        let open_conn = match self
            .open_connections
            .iter()
            .find(|candidate| crate::model::same_identity(&candidate.config, &conn))
        {
            Some(conn) => conn,
            None => {
                self.status = "Selected connection is not connected".to_string();
                return Ok(());
            }
        };

        execute!(terminal.backend_mut(), DisableMouseCapture).ok();
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        let result = run_ssh_terminal(&open_conn.session);

        execute!(terminal.backend_mut(), EnterAlternateScreen).ok();
        enable_raw_mode().ok();
        execute!(terminal.backend_mut(), EnableMouseCapture).ok();
        terminal.clear().ok();

        match result {
            Ok(()) => self.status = "Exited terminal session".to_string(),
            Err(err) => self.status = format!("Terminal session error: {err}"),
        }

        Ok(())
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('n') => {
                self.mode = Mode::NewConnection;
                self.new_connection = NewConnectionState::default();
                self.edit_index = None;
                self.new_connection_feedback = None;
                self.status = "Fill fields and press Enter to connect".to_string();
            }
            KeyCode::Char('e') => {
                if let Some(config) = self.connections.get(self.selected_saved).cloned() {
                    self.mode = Mode::NewConnection;
                    self.new_connection = self.prefill_new_connection(&config);
                    self.edit_index = Some(self.selected_saved);
                    self.new_connection_feedback = None;
                    self.status = "Edit fields and press Enter to save".to_string();
                } else {
                    self.status = "No saved connection selected".to_string();
                }
            }
            KeyCode::Char('c') => {
                if let Some(config) = self.connections.get(self.selected_saved).cloned() {
                    if let Err(err) = self.connect_and_open(config.clone()) {
                        self.record_connect_error(&config, &err);
                        self.status = format!("Connection failed: {err}");
                    }
                } else {
                    self.status = "No saved connection selected".to_string();
                }
            }
            KeyCode::Char('h') => {
                self.show_help = !self.show_help;
                self.show_header = !self.show_header;
            }
            KeyCode::Char('t') => {
                if self.open_connections.is_empty() {
                    self.status = "No open connections".to_string();
                } else {
                    self.pending_action = Some(AppAction::OpenTerminal);
                }
            }
            KeyCode::Char('f') => {
                if let Some(conn) = self.selected_connected_connection() {
                    self.start_transfer(conn);
                } else {
                    self.status = "Selected connection is not connected".to_string();
                }
            }
            KeyCode::Char('m') => {
                self.mode = Mode::ChangeMasterPassword;
                self.master_change = MasterPasswordState::default();
                self.status = "Update master password".to_string();
            }
            KeyCode::Char('d') => {
                if self.open_connections.is_empty() {
                    self.status = "No open connections".to_string();
                } else {
                    self.open_connections.remove(self.selected_tab);
                    if self.selected_tab > 0 {
                        self.selected_tab -= 1;
                    }
                    self.status = "Disconnected".to_string();
                }
            }
            KeyCode::Char('x') => {
                if self.connections.is_empty() {
                    self.status = "No saved connections".to_string();
                } else {
                    self.mode = Mode::ConfirmDelete;
                    self.delete_index = Some(self.selected_saved);
                    self.status = "Confirm delete".to_string();
                }
            }
            KeyCode::Tab => {
                if self.selected_saved + 1 < self.connections.len() {
                    self.selected_saved += 1;
                    self.history_page = 0;
                }
            }
            KeyCode::BackTab => {
                if self.selected_saved > 0 {
                    self.selected_saved -= 1;
                    self.history_page = 0;
                }
            }
            KeyCode::Up => {
                if self.selected_saved > 0 {
                    self.selected_saved -= 1;
                    self.history_page = 0;
                }
            }
            KeyCode::Down => {
                if self.selected_saved + 1 < self.connections.len() {
                    self.selected_saved += 1;
                    self.history_page = 0;
                }
            }
            KeyCode::Left => {
                if self.history_page > 0 {
                    self.history_page -= 1;
                }
            }
            KeyCode::Right => {
                if let Some(conn) = self.connections.get(self.selected_saved) {
                    let key = crate::model::connection_key(conn);
                    let has_error = self.last_error.contains_key(&key);
                    let max_page = self.max_history_page(conn.history.len(), has_error);
                    if self.history_page < max_page {
                        self.history_page += 1;
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_new_connection_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.file_picker.is_some() {
            return self.handle_file_picker_key(key);
        }
        if self.key_picker.is_some() {
            return self.handle_key_picker_key(key);
        }
        if self.try_result.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.try_result = None;
                }
                _ => {}
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.edit_index = None;
                self.new_connection_feedback = None;
                self.status = "Cancelled".to_string();
            }
            KeyCode::Tab | KeyCode::Down => self.advance_field(true),
            KeyCode::BackTab | KeyCode::Up => self.advance_field(false),
            KeyCode::Left | KeyCode::Right => {
                if self.new_connection.active_field == Field::AuthType {
                    let next = match (self.new_connection.auth_kind, key.code) {
                        (AuthKind::PasswordOnly, KeyCode::Right) => AuthKind::PrivateKey,
                        (AuthKind::PrivateKey, KeyCode::Right) => AuthKind::PrivateKeyWithPassword,
                        (AuthKind::PrivateKeyWithPassword, KeyCode::Right) => AuthKind::PasswordOnly,
                        (AuthKind::PasswordOnly, KeyCode::Left) => AuthKind::PrivateKeyWithPassword,
                        (AuthKind::PrivateKey, KeyCode::Left) => AuthKind::PasswordOnly,
                        (AuthKind::PrivateKeyWithPassword, KeyCode::Left) => AuthKind::PrivateKey,
                        (current, _) => current,
                    };
                    self.new_connection.auth_kind = next;
                }
            }
            KeyCode::F(2) => {
                if self.new_connection.active_field == Field::KeyPath {
                    self.open_file_picker()?;
                }
            }
            KeyCode::F(3) => {
                if self.new_connection.active_field == Field::KeyPath {
                    self.open_key_picker();
                }
            }
            KeyCode::Enter => {
                match self.new_connection.active_field {
                    Field::ActionTest => self.run_test_connection(),
                    Field::ActionSave => self.run_save_connection(),
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                self.edit_active_field(EditAction::Backspace);
            }
            KeyCode::Char(ch) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(false);
                }
                self.edit_active_field(EditAction::Insert(ch));
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_master_password_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = "Cancelled".to_string();
            }
            KeyCode::Tab => self.advance_master_field(true),
            KeyCode::BackTab => self.advance_master_field(false),
            KeyCode::Enter => match self.apply_master_password_change() {
                Ok(()) => {
                    self.mode = Mode::Normal;
                    self.status = "Master password updated".to_string();
                }
                Err(err) => {
                    self.status = format!("Master password not changed: {err}");
                }
            },
            KeyCode::Backspace => {
                self.edit_master_field(EditAction::Backspace);
            }
            KeyCode::Char(ch) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(false);
                }
                self.edit_master_field(EditAction::Insert(ch));
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_confirm_delete_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.mode = Mode::Normal;
                self.delete_index = None;
                self.status = "Cancelled".to_string();
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Some(index) = self.delete_index.take() {
                    if index < self.connections.len() {
                        let removed = self.connections.remove(index);
                        self.last_error.remove(&crate::model::connection_key(&removed));
                        self.save_store()?;
                        if self.selected_saved >= self.connections.len()
                            && self.selected_saved > 0
                        {
                            self.selected_saved -= 1;
                        }
                        self.status = "Connection removed".to_string();
                    }
                }
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        Ok(false)
    }

    fn build_new_config(&mut self) -> Result<ConnectionConfig> {
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
            user: self.new_connection.user.trim().to_string(),
            host: self.new_connection.host.trim().to_string(),
            auth,
            history: vec![],
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
        self.last_error.remove(&crate::model::connection_key(&config));
        self.status = format!("Connected to {}", config.label());
        Ok(())
    }

    fn save_or_connect(&mut self, mut config: ConnectionConfig) -> Result<()> {
        if let Some(index) = self.edit_index {
            if let Some(existing) = self.connections.get(index) {
                config.history = existing.history.clone();
            }
            self.connections.remove(index);
            self.upsert_connection(config);
            self.save_store()?;
            self.status = "Connection updated".to_string();
            return Ok(());
        }
        self.connect_and_open(config)
    }

    fn advance_field(&mut self, forward: bool) {
        let fields = self.active_fields();
        if let Some(pos) = fields.iter().position(|field| *field == self.new_connection.active_field)
        {
            let next = if forward {
                (pos + 1) % fields.len()
            } else if pos == 0 {
                fields.len() - 1
            } else {
                pos - 1
            };
            self.new_connection.active_field = fields[next];
        }
    }

    fn active_fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::User, Field::Host, Field::AuthType];
        match self.new_connection.auth_kind {
            AuthKind::PasswordOnly => {
                fields.push(Field::Password);
            }
            AuthKind::PrivateKey => fields.push(Field::KeyPath),
            AuthKind::PrivateKeyWithPassword => {
                fields.push(Field::KeyPath);
                fields.push(Field::Password);
            }
        }
        fields.push(Field::ActionTest);
        fields.push(Field::ActionSave);
        fields
    }

    fn edit_active_field(&mut self, action: EditAction) {
        let target = match self.new_connection.active_field {
            Field::User => &mut self.new_connection.user,
            Field::Host => &mut self.new_connection.host,
            Field::KeyPath => &mut self.new_connection.key_path,
            Field::Password => &mut self.new_connection.password,
            Field::ActionTest | Field::ActionSave => return,
            Field::AuthType => return,
        };
        match action {
            EditAction::Insert(ch) => target.push(ch),
            EditAction::Backspace => {
                target.pop();
            }
        }
    }

    fn advance_master_field(&mut self, forward: bool) {
        let fields = [MasterField::Current, MasterField::New, MasterField::Confirm];
        let pos = fields
            .iter()
            .position(|field| *field == self.master_change.active_field)
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % fields.len()
        } else if pos == 0 {
            fields.len() - 1
        } else {
            pos - 1
        };
        self.master_change.active_field = fields[next];
    }

    fn edit_master_field(&mut self, action: EditAction) {
        let target = match self.master_change.active_field {
            MasterField::Current => &mut self.master_change.current,
            MasterField::New => &mut self.master_change.new_password,
            MasterField::Confirm => &mut self.master_change.confirm,
        };
        match action {
            EditAction::Insert(ch) => target.push(ch),
            EditAction::Backspace => {
                target.pop();
            }
        }
    }

    fn apply_master_password_change(&mut self) -> Result<()> {
        if self.master_change.current.is_empty() {
            anyhow::bail!("Current password is required");
        }
        if self.master_change.new_password.is_empty() {
            anyhow::bail!("New password is required");
        }
        if self.master_change.new_password != self.master_change.confirm {
            anyhow::bail!("New password confirmation does not match");
        }

        let salt = Base64.decode(&self.master.salt_b64).context("decode salt")?;
        let current_key = crate::storage::derive_key(&self.master_change.current, &salt);
        let check = crate::storage::decrypt_string(&self.master.check, &current_key)
            .context("verify current password")?;
        if check != "ssh-client-check" {
            anyhow::bail!("Current master password incorrect");
        }

        let (new_master, new_key) =
            create_master_from_password(&self.master_change.new_password)?;
        let stored = crate::model::StoreFile {
            master: new_master.clone(),
            connections: self
                .connections
                .iter()
                .map(|conn| crate::storage::encrypt_connection(conn, &new_key))
                .collect::<Result<Vec<_>>>()?,
        };
        save_store(&self.config_path, &stored)?;
        self.master = new_master;
        self.master_key = new_key;
        self.master_change = MasterPasswordState::default();
        Ok(())
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

    fn save_store(&self) -> Result<()> {
        let stored = crate::model::StoreFile {
            master: self.master.clone(),
            connections: self
                .connections
                .iter()
                .map(|conn| crate::storage::encrypt_connection(conn, &self.master_key))
                .collect::<Result<Vec<_>>>()?,
        };
        save_store(&self.config_path, &stored)
    }

    fn open_file_picker(&mut self) -> Result<()> {
        let start_dir = resolve_picker_start(&self.new_connection.key_path)?;
        let entries = read_dir_entries(&start_dir)?;
        self.file_picker = Some(FilePickerState {
            cwd: start_dir,
            entries,
            selected: 0,
        });
        Ok(())
    }

    fn open_key_picker(&mut self) {
        let keys = self.collect_key_candidates();
        if keys.is_empty() {
            self.status = "No known keys yet".to_string();
            return;
        }
        self.key_picker = Some(KeyPickerState { keys, selected: 0 });
    }

    fn handle_file_picker_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(picker) = &mut self.file_picker {
            match key.code {
                KeyCode::Esc => {
                    self.file_picker = None;
                    if self.transfer.is_some() {
                        self.transfer = None;
                    }
                }
                KeyCode::Up => {
                    if picker.selected > 0 {
                        picker.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    if picker.selected + 1 < picker.entries.len() {
                        picker.selected += 1;
                    }
                }
                KeyCode::Backspace => {
                    if let Some(parent) = picker.cwd.parent() {
                        picker.cwd = parent.to_path_buf();
                        picker.entries = read_dir_entries(&picker.cwd)?;
                        picker.selected = 0;
                    }
                }
                KeyCode::Enter => {
                    if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                        if entry.is_dir {
                            if self.transfer.as_ref().is_some_and(|t| t.step == TransferStep::PickSource) {
                                picker.cwd = entry.path;
                                picker.entries = read_dir_entries(&picker.cwd)?;
                                picker.selected = 0;
                            } else {
                                picker.cwd = entry.path;
                                picker.entries = read_dir_entries(&picker.cwd)?;
                                picker.selected = 0;
                            }
                        } else {
                            if self.transfer.as_ref().is_some_and(|t| t.step == TransferStep::PickSource) {
                                self.select_source(entry.path, false);
                                self.file_picker = None;
                                self.open_remote_picker()?;
                            } else {
                                self.new_connection.key_path =
                                    entry.path.to_string_lossy().into_owned();
                                self.file_picker = None;
                            }
                        }
                    }
                }
                KeyCode::Char('s') => {
                    if self.transfer.as_ref().is_some_and(|t| t.step == TransferStep::PickSource) {
                        if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                            if entry.is_dir {
                                self.select_source(entry.path, true);
                                self.file_picker = None;
                                self.open_remote_picker()?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    fn handle_remote_picker_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut picker) = self.remote_picker.take() else {
            return Ok(false);
        };
        match key.code {
            KeyCode::Esc => {
                self.remote_picker = None;
                self.transfer = None;
                return Ok(false);
            }
            KeyCode::Up => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down => {
                if picker.selected + 1 < picker.entries.len() {
                    picker.selected += 1;
                }
            }
            KeyCode::Backspace => {
                if picker.cwd != "/" {
                    let new_cwd = picker
                        .cwd
                        .trim_end_matches('/')
                        .rsplit_once('/')
                        .map(|(base, _)| if base.is_empty() { "/".to_string() } else { base.to_string() })
                        .unwrap_or_else(|| "/".to_string());
                    picker.cwd = new_cwd.clone();
                    picker.entries.clear();
                    picker.selected = 0;
                    picker.loading = true;
                    picker.error = None;
                    self.start_remote_fetch(new_cwd)?;
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                    if entry.is_dir {
                        let new_cwd = entry.path;
                        picker.cwd = new_cwd.clone();
                        picker.entries.clear();
                        picker.selected = 0;
                        picker.loading = true;
                        picker.error = None;
                        self.start_remote_fetch(new_cwd)?;
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                    if entry.is_dir {
                        self.select_target_dir(entry.path);
                        self.transfer = self.transfer.take().map(|mut t| {
                            t.step = TransferStep::Confirm;
                            t
                        });
                        return Ok(false);
                    }
                }
            }
            _ => {}
        }
        self.remote_picker = Some(picker);
        Ok(false)
    }

    fn handle_transfer_confirm(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.transfer = None;
                self.status = "Cancelled".to_string();
            }
            KeyCode::Enter => {
                if let Err(err) = self.execute_transfer() {
                    self.notice = Some(Notice {
                        title: "Transfer failed".to_string(),
                        message: err.to_string(),
                    });
                } else {
                    self.notice = Some(Notice {
                        title: "Transfer complete".to_string(),
                        message: "Transfer finished successfully".to_string(),
                    });
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_key_picker_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(picker) = &mut self.key_picker {
            match key.code {
                KeyCode::Esc => {
                    self.key_picker = None;
                }
                KeyCode::Up => {
                    if picker.selected > 0 {
                        picker.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    if picker.selected + 1 < picker.keys.len() {
                        picker.selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(entry) = picker.keys.get(picker.selected).cloned() {
                        self.new_connection.key_path = entry.path;
                        if let Some(pass) = entry.password {
                            self.new_connection.password = pass;
                            self.new_connection.auth_kind = AuthKind::PrivateKeyWithPassword;
                        }
                        self.key_picker = None;
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    fn record_connect_error(&mut self, config: &ConnectionConfig, err: &anyhow::Error) {
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
                self.status = format!("Failed to save history: {err}");
            }
        }
    }

    fn try_connect(&self, config: &ConnectionConfig) -> Result<()> {
        let _session = connect_ssh(config)?;
        Ok(())
    }

    pub(crate) fn set_details_height(&mut self, height: u16) {
        self.details_height = height;
    }

    pub(crate) fn history_range(
        &self,
        history_len: usize,
        has_error: bool,
    ) -> (usize, usize) {
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

    fn max_history_page(&self, history_len: usize, has_error: bool) -> usize {
        let page_size = self.history_page_size(has_error);
        if history_len == 0 {
            return 0;
        }
        (history_len - 1) / page_size
    }

    fn history_page_size(&self, has_error: bool) -> usize {
        let inner_height = self.details_height.saturating_sub(2) as usize;
        let base_lines = 4 + usize::from(has_error);
        let pre_history = base_lines + 2;
        inner_height.saturating_sub(pre_history).max(1)
    }

    fn run_test_connection(&mut self) {
        match self.build_new_config() {
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

    fn run_save_connection(&mut self) {
        match self.build_new_config() {
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
                        self.new_connection_feedback =
                            Some(format!("Connection failed: {err}"));
                    }
                }
            }
            Err(err) => {
                self.new_connection_feedback = Some(format!("Missing fields: {err}"));
            }
        }
    }

    fn prefill_new_connection(&self, config: &ConnectionConfig) -> NewConnectionState {
        let mut state = NewConnectionState::default();
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

    fn start_transfer(&mut self, conn: ConnectionConfig) {
        self.transfer = Some(TransferState {
            step: TransferStep::PickSource,
            source_path: None,
            source_is_dir: false,
            target_dir: None,
        });
        if let Ok(start_dir) = resolve_picker_start("") {
            if let Ok(entries) = read_dir_entries(&start_dir) {
                self.file_picker = Some(FilePickerState {
                    cwd: start_dir,
                    entries,
                    selected: 0,
                });
            }
        }
        self.status = format!("Select source for {}", conn.label());
    }

    fn select_source(&mut self, path: PathBuf, is_dir: bool) {
        if let Some(transfer) = &mut self.transfer {
            transfer.source_path = Some(path);
            transfer.source_is_dir = is_dir;
            transfer.step = TransferStep::PickTarget;
        }
    }

    fn open_remote_picker(&mut self) -> Result<()> {
        let cwd = "/".to_string();
        self.remote_picker = Some(RemotePickerState {
            cwd: cwd.clone(),
            entries: vec![],
            selected: 0,
            loading: true,
            error: None,
        });
        self.start_remote_fetch(cwd)?;
        Ok(())
    }

    fn select_target_dir(&mut self, path: String) {
        if let Some(transfer) = &mut self.transfer {
            transfer.target_dir = Some(path);
            transfer.step = TransferStep::Confirm;
        }
    }

    fn execute_transfer(&mut self) -> Result<()> {
        let Some(transfer) = self.transfer.take() else {
            return Ok(());
        };
        let Some(source) = transfer.source_path else {
            anyhow::bail!("Missing source");
        };
        let Some(target_dir) = transfer.target_dir else {
            anyhow::bail!("Missing target");
        };
        let Some(conn) = self.selected_connected_connection() else {
            anyhow::bail!("Selected connection is not connected");
        };
        let open_conn = self
            .open_connections
            .iter()
            .find(|candidate| crate::model::same_identity(&candidate.config, &conn))
            .context("selected connection is not connected")?;
        crate::ssh::transfer_path(&open_conn.session, &source, &target_dir, transfer.source_is_dir)?;
        self.status = "Transfer completed".to_string();
        Ok(())
    }

    fn selected_connected_connection(&self) -> Option<ConnectionConfig> {
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

    fn start_remote_fetch(&mut self, cwd: String) -> Result<()> {
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
                entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
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
                        picker.entries = entries;
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

enum EditAction {
    Insert(char),
    Backspace,
}

fn resolve_picker_start(current: &str) -> Result<PathBuf> {
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

fn read_dir_entries(dir: &Path) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).context("read dir")? {
        let entry = entry.context("read dir entry")?;
        let path = entry.path();
        let file_type = entry.file_type().context("read file type")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        entries.push(FileEntry {
            name,
            path,
            is_dir: file_type.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}
