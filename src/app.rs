use std::collections::{HashMap, VecDeque};
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
    NewConnectionState, Notice, OpenConnection, RemoteEntry, RemotePickerState, TransferDirection,
    TransferState, TransferStep, TransferUpdate, TryResult,
};
use crate::ssh::{connect_ssh, expand_tilde, run_ssh_terminal};
use crate::storage::{
    config_path, create_master_from_password, load_or_init_store, log_path, save_store,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderMode {
    Help,
    Logs,
    Off,
}

#[derive(Debug, Clone, Copy)]
enum NoticeAction {
    ConnectTerminal,
    ConnectUpload,
    ConnectDownload,
}

pub(crate) struct App {
    pub(crate) config_path: PathBuf,
    pub(crate) log_path: PathBuf,
    pub(crate) last_log: String,
    pub(crate) log_lines: VecDeque<String>,
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
    notice_action: Option<NoticeAction>,
    pub(crate) header_mode: HeaderMode,
    pub(crate) history_page: usize,
    pub(crate) details_height: u16,
    pub(crate) transfer: Option<TransferState>,
    pub(crate) remote_picker: Option<RemotePickerState>,
    remote_fetch: Option<mpsc::Receiver<Result<Vec<RemoteEntry>>>>,
    transfer_progress: Option<mpsc::Receiver<TransferUpdate>>,
    transfer_cancel: Option<mpsc::Sender<()>>,
    pub(crate) transfer_hidden: bool,
    transfer_last_logged: u64,
}

impl App {
    pub(crate) fn load_with_master() -> Result<Self> {
        let config_path = config_path()?;
        let (master, master_key, connections) = load_or_init_store(&config_path)?;
        let log_path = log_path()?;
        prune_log_file(&log_path, 7, 10_000);
        let log_lines = VecDeque::new();
        let last_log = String::from("No logs yet");
        let mut app = Self {
            config_path,
            log_path,
            last_log,
            log_lines,
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
            notice_action: None,
            header_mode: HeaderMode::Help,
            history_page: 0,
            details_height: 0,
            transfer: None,
            remote_picker: None,
            remote_fetch: None,
            transfer_progress: None,
            transfer_cancel: None,
            transfer_hidden: false,
            transfer_last_logged: 0,
        };
        app.sort_connections_by_recent(None);
        app.set_status("Ready");
        Ok(app)
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.notice.is_some() {
            if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                self.notice = None;
                if matches!(key.code, KeyCode::Enter) {
                    if let Some(action) = self.notice_action.take() {
                        if let Some(config) = self.connect_selected() {
                            match action {
                                NoticeAction::ConnectTerminal => {
                                    self.pending_action = Some(AppAction::OpenTerminal);
                                }
                                NoticeAction::ConnectUpload => {
                                    self.start_upload(config);
                                }
                                NoticeAction::ConnectDownload => {
                                    self.start_download(config);
                                }
                            }
                        }
                    }
                }
            } else if matches!(key.code, KeyCode::Char('c')) {
                self.notice = None;
                self.notice_action = None;
                self.connect_selected();
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
            if self
                .transfer
                .as_ref()
                .is_some_and(|t| t.step == TransferStep::Transferring)
                && self.transfer_hidden
            {
                // Allow normal navigation while transfer runs in background.
            } else if matches!(
                self.transfer.as_ref().map(|t| t.step),
                Some(TransferStep::Confirm | TransferStep::Transferring)
            ) {
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

    pub(crate) fn notice_action_label(&self) -> Option<&'static str> {
        match self.notice_action {
            Some(NoticeAction::ConnectTerminal) => Some("connect and open the terminal"),
            Some(NoticeAction::ConnectUpload) => Some("connect and select what to upload"),
            Some(NoticeAction::ConnectDownload) => Some("connect and select what to download"),
            None => None,
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.log_line(&message);
    }

    fn log_line(&mut self, message: &str) {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let line = format!("{timestamp} | {message}");
        if let Some(parent) = self.log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
        self.last_log = line.clone();
        self.log_lines.push_back(line);
        while self.log_lines.len() > 100 {
            self.log_lines.pop_front();
        }
    }

    fn cycle_header_mode(&mut self) {
        self.header_mode = match self.header_mode {
            HeaderMode::Help => HeaderMode::Logs,
            HeaderMode::Logs => HeaderMode::Off,
            HeaderMode::Off => HeaderMode::Help,
        };
    }

    pub(crate) fn handle_terminal_mode(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            self.set_status("Selected connection is not connected");
            return Ok(());
        };
        let open_conn = match self
            .open_connections
            .iter()
            .find(|candidate| crate::model::same_identity(&candidate.config, &conn))
        {
            Some(conn) => conn,
            None => {
                self.set_status("Selected connection is not connected");
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
            Ok(()) => self.set_status("Exited terminal session"),
            Err(err) => self.set_status(format!("Terminal session error: {err}")),
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
                self.set_status("Fill fields and press Enter to connect");
            }
            KeyCode::Char('e') => {
                if let Some(config) = self.connections.get(self.selected_saved).cloned() {
                    self.mode = Mode::NewConnection;
                    self.new_connection = self.prefill_new_connection(&config);
                    self.edit_index = Some(self.selected_saved);
                    self.new_connection_feedback = None;
                    self.set_status("Edit fields and press Enter to save");
                } else {
                    self.set_status("No saved connection selected");
                }
            }
            KeyCode::Char('c') => {
                if self.selected_connected_connection().is_some() {
                    self.disconnect_selected();
                } else {
                    self.connect_selected();
                }
            }
            KeyCode::Char('h') => {
                self.cycle_header_mode();
            }
            KeyCode::Char('t') => {
                if self.selected_connected_connection().is_some() {
                    self.pending_action = Some(AppAction::OpenTerminal);
                } else {
                    self.notice = Some(Notice {
                        title: "Not connected".to_string(),
                        message: "Please connect to the host machine first.".to_string(),
                    });
                    self.notice_action = Some(NoticeAction::ConnectTerminal);
                }
            }
            KeyCode::Char('u') => {
                if let Some(conn) = self.selected_connected_connection() {
                    self.start_upload(conn);
                } else {
                    self.notice = Some(Notice {
                        title: "Not connected".to_string(),
                        message: "Please connect to the host machine first.".to_string(),
                    });
                    self.notice_action = Some(NoticeAction::ConnectUpload);
                }
            }
            KeyCode::Char('d') => {
                if let Some(conn) = self.selected_connected_connection() {
                    self.start_download(conn);
                } else {
                    self.notice = Some(Notice {
                        title: "Not connected".to_string(),
                        message: "Please connect to the host machine first.".to_string(),
                    });
                    self.notice_action = Some(NoticeAction::ConnectDownload);
                }
            }
            KeyCode::Char('o') => {
                self.mode = Mode::ChangeMasterPassword;
                self.master_change = MasterPasswordState::default();
                self.set_status("Update master password");
            }
            KeyCode::Char('x') => {
                if self.connections.is_empty() {
                    self.set_status("No saved connections");
                } else {
                    self.mode = Mode::ConfirmDelete;
                    self.delete_index = Some(self.selected_saved);
                    self.set_status("Confirm delete");
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
                self.set_status("Cancelled");
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
                self.set_status("Cancelled");
            }
            KeyCode::Tab | KeyCode::Down => self.advance_master_field(true),
            KeyCode::BackTab | KeyCode::Up => self.advance_master_field(false),
            KeyCode::Enter => {
                if self.master_change.active_field == MasterField::ActionSave {
                    match self.apply_master_password_change() {
                        Ok(()) => {
                            self.mode = Mode::Normal;
                            self.set_status("Master password updated");
                        }
                        Err(err) => {
                            self.set_status(format!("Master password not changed: {err}"));
                        }
                    }
                }
            }
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
                self.set_status("Cancelled");
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
                        self.set_status("Connection removed");
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
        let name = if self.new_connection.name.trim().is_empty() {
            self.new_connection.host.trim().to_string()
        } else {
            self.new_connection.name.trim().to_string()
        };

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
            name,
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
        self.sort_connections_by_recent(Some(crate::model::connection_key(&config)));
        self.last_error.remove(&crate::model::connection_key(&config));
        self.set_status(format!("Connected to {}", config.label()));
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
            self.set_status("Connection updated");
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
        let mut fields = vec![Field::Name, Field::User, Field::Host, Field::AuthType];
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
            Field::Name => &mut self.new_connection.name,
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
        let fields = [
            MasterField::Current,
            MasterField::New,
            MasterField::Confirm,
            MasterField::ActionSave,
        ];
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
            MasterField::ActionSave => return,
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

    fn connect_selected(&mut self) -> Option<ConnectionConfig> {
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

    fn disconnect_selected(&mut self) {
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
            self.set_status("Selected connection is not connected");
        }
    }

    fn sort_connections_by_recent(&mut self, selected_key: Option<String>) {
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

    fn open_file_picker(&mut self) -> Result<()> {
        let start_dir = resolve_picker_start(&self.new_connection.key_path)?;
        let entries = read_dir_entries_filtered(&start_dir, false)?;
        self.file_picker = Some(FilePickerState {
            cwd: start_dir,
            entries,
            selected: 0,
        });
        Ok(())
    }

    fn open_local_picker(&mut self, start: Option<PathBuf>, only_dirs: bool) -> Result<()> {
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

    fn open_key_picker(&mut self) {
        let keys = self.collect_key_candidates();
        if keys.is_empty() {
            self.set_status("No known keys yet");
            return;
        }
        self.key_picker = Some(KeyPickerState { keys, selected: 0 });
    }

    fn handle_file_picker_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(picker) = &mut self.file_picker {
            let transfer_mode = self
                .transfer
                .as_ref()
                .map(|t| (t.direction, t.step));
            let only_dirs = transfer_mode
                .map(|m| m.0 == TransferDirection::Download && m.1 == TransferStep::PickTarget)
                .unwrap_or(false);
            match key.code {
                KeyCode::Esc => {
                    self.file_picker = None;
                    if self.transfer.is_some() {
                        self.transfer = None;
                    }
                }
                KeyCode::Char('b') => {
                    if let Some((direction, step)) = transfer_mode {
                        match (direction, step) {
                            (TransferDirection::Upload, TransferStep::PickSource) => {
                                self.set_status("Already at source selection");
                            }
                            (TransferDirection::Download, TransferStep::PickTarget) => {
                                let start = self
                                    .transfer
                                    .as_ref()
                                    .and_then(|t| t.source_remote.as_ref())
                                    .map(|path| {
                                        Path::new(path)
                                            .parent()
                                            .map(|p| p.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| "/".to_string())
                                    });
                                if let Some(transfer) = &mut self.transfer {
                                    transfer.step = TransferStep::PickSource;
                                }
                                self.file_picker = None;
                                if let Some(start) = start {
                                    self.open_remote_picker_at(start, true)?;
                                } else {
                                    self.open_remote_picker()?;
                                }
                            }
                            _ => {}
                        }
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
                        picker.entries = read_dir_entries_filtered(&picker.cwd, transfer_mode.map(|m| m.0 == TransferDirection::Download && m.1 == TransferStep::PickTarget).unwrap_or(false))?;
                        picker.selected = 0;
                    }
                }
                KeyCode::Enter => {
                    if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                        if entry.is_dir {
                            if only_dirs {
                                let subdirs = read_dir_entries_filtered(&entry.path, true)?;
                                if subdirs.is_empty() {
                                    self.notice = Some(Notice {
                                        title: "No subfolders".to_string(),
                                        message: "This folder has no subfolders. To select it as the target, press S.".to_string(),
                                    });
                                    return Ok(false);
                                }
                            }
                            picker.cwd = entry.path;
                            picker.entries = read_dir_entries_filtered(&picker.cwd, only_dirs)?;
                            picker.selected = 0;
                        } else {
                            match transfer_mode {
                                Some((TransferDirection::Upload, TransferStep::PickSource)) => {
                                    self.select_source(entry.path, false);
                                    self.file_picker = None;
                                    self.open_remote_picker()?;
                                }
                                Some((TransferDirection::Download, TransferStep::PickTarget)) => {}
                                _ => {
                                    self.new_connection.key_path =
                                        entry.path.to_string_lossy().into_owned();
                                    self.file_picker = None;
                                }
                            }
                        }
                    }
                }
                KeyCode::Char('s') => {
                    if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                        if entry.is_dir {
                            match transfer_mode {
                                Some((TransferDirection::Upload, TransferStep::PickSource)) => {
                                    self.select_source(entry.path, true);
                                    self.file_picker = None;
                                    self.open_remote_picker()?;
                                }
                                Some((TransferDirection::Download, TransferStep::PickTarget)) => {
                                    self.select_target_local_dir(entry.path);
                                    self.file_picker = None;
                                }
                                _ => {}
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
        let transfer_mode = self
            .transfer
            .as_ref()
            .map(|t| (t.direction, t.step));
        let only_dirs = picker.only_dirs;
        match key.code {
            KeyCode::Esc => {
                self.remote_picker = None;
                self.transfer = None;
                return Ok(false);
            }
            KeyCode::Char('b') => {
                if let Some((direction, step)) = transfer_mode {
                    match (direction, step) {
                        (TransferDirection::Upload, TransferStep::PickTarget) => {
                            let start = self
                                .transfer
                                .as_ref()
                                .and_then(|t| t.source_path.as_ref())
                                .and_then(|p| p.parent())
                                .map(|p| p.to_path_buf());
                            if let Some(transfer) = &mut self.transfer {
                                transfer.step = TransferStep::PickSource;
                            }
                            self.remote_picker = None;
                            self.open_local_picker(start, false)?;
                            return Ok(false);
                        }
                        (TransferDirection::Download, TransferStep::PickSource) => {
                            self.set_status("Already at source selection");
                            return Ok(false);
                        }
                        _ => {}
                    }
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
                    self.start_remote_fetch(new_cwd, picker.only_dirs)?;
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                    if entry.is_dir {
                        if only_dirs {
                            let Some(conn) = self.selected_connected_connection() else {
                                self.set_status("Selected connection is not connected");
                                self.remote_picker = Some(picker);
                                return Ok(false);
                            };
                            let open_conn = match self
                                .open_connections
                                .iter()
                                .find(|candidate| crate::model::same_identity(&candidate.config, &conn))
                            {
                                Some(conn) => conn,
                                None => {
                                    self.set_status("Selected connection is not connected");
                                    self.remote_picker = Some(picker);
                                    return Ok(false);
                                }
                            };
                            if !crate::ssh::remote_has_subdirs(&open_conn.session, &entry.path)? {
                                self.notice = Some(Notice {
                                    title: "No subfolders".to_string(),
                                    message: "This folder has no subfolders. To select it as the target, press S.".to_string(),
                                });
                                self.remote_picker = Some(picker);
                                return Ok(false);
                            }
                        }
                        let new_cwd = entry.path;
                        picker.cwd = new_cwd.clone();
                        picker.entries.clear();
                        picker.selected = 0;
                        picker.loading = true;
                        picker.error = None;
                        self.start_remote_fetch(new_cwd, picker.only_dirs)?;
                    } else if matches!(
                        transfer_mode,
                        Some((TransferDirection::Download, TransferStep::PickSource))
                    ) {
                        self.select_remote_source(entry.path, false);
                        self.remote_picker = None;
                        self.open_local_target_picker()?;
                        return Ok(false);
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                    if entry.is_dir {
                        match transfer_mode {
                            Some((TransferDirection::Upload, TransferStep::PickTarget)) => {
                                self.select_target_dir(entry.path);
                                self.transfer = self.transfer.take().map(|mut t| {
                                    t.step = TransferStep::Confirm;
                                    t
                                });
                                return Ok(false);
                            }
                            Some((TransferDirection::Download, TransferStep::PickSource)) => {
                                self.select_remote_source(entry.path, true);
                                self.remote_picker = None;
                                self.open_local_target_picker()?;
                                return Ok(false);
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        self.remote_picker = Some(picker);
        Ok(false)
    }

    fn handle_transfer_confirm(&mut self, key: KeyEvent) -> Result<bool> {
        if matches!(
            self.transfer.as_ref().map(|t| t.step),
            Some(TransferStep::Transferring)
        ) {
            match key.code {
                KeyCode::Esc => {
                    if let Some(cancel) = self.transfer_cancel.take() {
                        let _ = cancel.send(());
                    }
                }
                KeyCode::Enter => {
                    self.transfer_hidden = true;
                }
                _ => {}
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Esc => {
                self.transfer = None;
                self.set_status("Cancelled");
            }
            KeyCode::Enter => {
                self.start_transfer_job();
            }
            KeyCode::Char('b') => {
                if let Some(transfer) = &mut self.transfer {
                    transfer.step = TransferStep::PickTarget;
                }
                match self.transfer.as_ref().map(|t| t.direction) {
                    Some(TransferDirection::Upload) => {
                        let start = self
                            .transfer
                            .as_ref()
                            .and_then(|t| t.target_dir.as_ref())
                            .cloned()
                            .unwrap_or_else(|| "/".to_string());
                        self.open_remote_picker_at(start, true)?;
                    }
                    Some(TransferDirection::Download) => {
                        let start = self
                            .transfer
                            .as_ref()
                            .and_then(|t| t.target_local_dir.as_ref())
                            .map(|p| p.to_path_buf());
                        self.open_local_target_picker_at(start)?;
                    }
                    None => {}
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn start_transfer_job(&mut self) {
        let Some(transfer) = self.transfer.take() else {
            return;
        };
        let Some(config) = self.selected_connected_connection() else {
            self.set_status("Selected connection is not connected");
            return;
        };
        let (tx, rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let transfer_clone = transfer.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let session = connect_ssh(&config)?;
                match transfer_clone.direction {
                    TransferDirection::Upload => {
                        let Some(source) = transfer_clone.source_path else {
                            anyhow::bail!("Missing source");
                        };
                        let Some(target_dir) = transfer_clone.target_dir else {
                            anyhow::bail!("Missing target");
                        };
                        crate::ssh::transfer_path_with_progress(
                            &session,
                            &source,
                            &target_dir,
                            transfer_clone.source_is_dir,
                            &tx,
                            &cancel_rx,
                        )?;
                    }
                    TransferDirection::Download => {
                        let Some(source) = transfer_clone.source_remote else {
                            anyhow::bail!("Missing source");
                        };
                        let Some(target_dir) = transfer_clone.target_local_dir else {
                            anyhow::bail!("Missing target");
                        };
                        crate::ssh::download_path_with_progress(
                            &session,
                            &source,
                            &target_dir,
                            transfer_clone.source_is_dir,
                            &tx,
                            &cancel_rx,
                        )?;
                    }
                }
                Ok(())
            })();
            let _ = tx.send(TransferUpdate::Done(result.map_err(|err| err.to_string())));
        });
        let mut transfer = transfer;
        transfer.step = TransferStep::Transferring;
        transfer.progress_bytes = 0;
        self.transfer = Some(transfer);
        self.transfer_progress = Some(rx);
        self.transfer_cancel = Some(cancel_tx);
        self.transfer_hidden = false;
        self.transfer_last_logged = 0;
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
                self.set_status(format!("Failed to save history: {err}"));
            }
            self.sort_connections_by_recent(Some(crate::model::connection_key(config)));
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

    fn start_transfer(&mut self, conn: ConnectionConfig) {
        self.transfer = Some(TransferState {
            direction: TransferDirection::Upload,
            step: TransferStep::PickSource,
            source_path: None,
            source_remote: None,
            source_is_dir: false,
            target_dir: None,
            target_local_dir: None,
            size_bytes: None,
            progress_bytes: 0,
        });
        if let Ok(start_dir) = resolve_picker_start("") {
            if let Ok(entries) = read_dir_entries_filtered(&start_dir, false) {
                self.file_picker = Some(FilePickerState {
                    cwd: start_dir,
                    entries,
                    selected: 0,
                });
            }
        }
        self.set_status(format!("Select source for {}", conn.label()));
    }

    fn start_upload(&mut self, conn: ConnectionConfig) {
        self.start_transfer(conn);
    }

    fn start_download(&mut self, conn: ConnectionConfig) {
        self.transfer = Some(TransferState {
            direction: TransferDirection::Download,
            step: TransferStep::PickSource,
            source_path: None,
            source_remote: None,
            source_is_dir: false,
            target_dir: None,
            target_local_dir: None,
            size_bytes: None,
            progress_bytes: 0,
        });
        if let Err(err) = self.open_remote_picker() {
            self.set_status(format!("Failed to open remote picker: {err}"));
        }
        self.set_status(format!("Select remote source for {}", conn.label()));
    }

    fn select_source(&mut self, path: PathBuf, is_dir: bool) {
        if let Some(transfer) = &mut self.transfer {
            transfer.source_path = Some(path);
            transfer.source_is_dir = is_dir;
            transfer.step = TransferStep::PickTarget;
            transfer.size_bytes = None;
        }
    }

    fn select_remote_source(&mut self, path: String, is_dir: bool) {
        if let Some(transfer) = &mut self.transfer {
            transfer.source_remote = Some(path);
            transfer.source_is_dir = is_dir;
            transfer.step = TransferStep::PickTarget;
            transfer.size_bytes = None;
        }
    }

    fn open_remote_picker(&mut self) -> Result<()> {
        let cwd = if let Some(conn) = self.selected_connected_connection() {
            format!("/home/{}", conn.user)
        } else {
            "/".to_string()
        };
        let only_dirs = self
            .transfer
            .as_ref()
            .is_some_and(|t| t.direction == TransferDirection::Upload && t.step == TransferStep::PickTarget);
        self.open_remote_picker_at(cwd, only_dirs)
    }

    fn open_remote_picker_at(&mut self, cwd: String, only_dirs: bool) -> Result<()> {
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

    fn open_local_target_picker(&mut self) -> Result<()> {
        self.open_local_target_picker_at(None)
    }

    fn open_local_target_picker_at(&mut self, start: Option<PathBuf>) -> Result<()> {
        self.open_local_picker(start, true)
    }

    fn select_target_dir(&mut self, path: String) {
        if let Some(transfer) = &mut self.transfer {
            transfer.target_dir = Some(path);
            transfer.step = TransferStep::Confirm;
            if transfer.size_bytes.is_none() {
                transfer.size_bytes =
                    compute_local_size(&transfer.source_path, transfer.source_is_dir).ok();
            }
        }
    }

    fn select_target_local_dir(&mut self, path: PathBuf) {
        let size = if let Some(transfer) = &self.transfer {
            if transfer.size_bytes.is_none() {
                self.compute_remote_size(transfer.source_remote.as_ref(), transfer.source_is_dir)
                    .ok()
            } else {
                None
            }
        } else {
            None
        };
        if let Some(transfer) = &mut self.transfer {
            transfer.target_local_dir = Some(path);
            transfer.step = TransferStep::Confirm;
            if transfer.size_bytes.is_none() {
                transfer.size_bytes = size;
            }
        }
    }

    fn execute_transfer(&mut self) -> Result<()> {
        let Some(transfer) = self.transfer.take() else {
            return Ok(());
        };
        let Some(conn) = self.selected_connected_connection() else {
            anyhow::bail!("Selected connection is not connected");
        };
        let open_conn = self
            .open_connections
            .iter()
            .find(|candidate| crate::model::same_identity(&candidate.config, &conn))
            .context("selected connection is not connected")?;
        match transfer.direction {
            TransferDirection::Upload => {
                let Some(source) = transfer.source_path else {
                    anyhow::bail!("Missing source");
                };
                let Some(target_dir) = transfer.target_dir else {
                    anyhow::bail!("Missing target");
                };
                crate::ssh::transfer_path(
                    &open_conn.session,
                    &source,
                    &target_dir,
                    transfer.source_is_dir,
                )?;
            }
            TransferDirection::Download => {
                let Some(source) = transfer.source_remote else {
                    anyhow::bail!("Missing source");
                };
                let Some(target_dir) = transfer.target_local_dir else {
                    anyhow::bail!("Missing target");
                };
                crate::ssh::download_path(
                    &open_conn.session,
                    &source,
                    &target_dir,
                    transfer.source_is_dir,
                )?;
            }
        }
        self.set_status("Transfer completed");
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

    fn start_remote_fetch(&mut self, cwd: String, only_dirs: bool) -> Result<()> {
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

    pub(crate) fn poll_transfer_progress(&mut self) {
        let Some(rx) = self.transfer_progress.take() else {
            return;
        };
        let mut done = false;
        while let Ok(update) = rx.try_recv() {
            match update {
                TransferUpdate::Bytes(amount) => {
                    let mut log_message = None;
                    let mut new_progress = None;
                    if let Some(transfer) = &mut self.transfer {
                        transfer.progress_bytes = transfer.progress_bytes.saturating_add(amount);
                        let threshold = 1024 * 1024;
                        if transfer
                            .progress_bytes
                            .saturating_sub(self.transfer_last_logged)
                            >= threshold
                        {
                            let total = transfer.size_bytes.unwrap_or(0);
                            log_message = Some(if total == 0 {
                                format!("Transfer progress: {}", transfer.progress_bytes)
                            } else {
                                format!(
                                    "Transfer progress: {} / {}",
                                    transfer.progress_bytes,
                                    total
                                )
                            });
                            new_progress = Some(transfer.progress_bytes);
                        }
                    }
                    if let Some(message) = log_message {
                        self.log_line(&message);
                    }
                    if let Some(progress) = new_progress {
                        self.transfer_last_logged = progress;
                    }
                }
                TransferUpdate::Done(result) => {
                    match result {
                        Ok(()) => {
                            self.notice = Some(Notice {
                                title: "Transfer complete".to_string(),
                                message: "Transfer finished successfully".to_string(),
                            });
                        }
                        Err(err) => {
                            self.notice = Some(Notice {
                                title: "Transfer failed".to_string(),
                                message: err,
                            });
                        }
                    }
                    self.transfer = None;
                    self.transfer_cancel = None;
                    done = true;
                }
            }
        }
        if !done {
            self.transfer_progress = Some(rx);
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

    fn compute_remote_size(&self, path: Option<&String>, is_dir: bool) -> Result<u64> {
        let Some(path) = path else {
            anyhow::bail!("missing remote source");
        };
        let Some(conn) = self.selected_connected_connection() else {
            anyhow::bail!("Selected connection is not connected");
        };
        let open_conn = self
            .open_connections
            .iter()
            .find(|candidate| crate::model::same_identity(&candidate.config, &conn))
            .context("selected connection is not connected")?;
        crate::ssh::remote_size(&open_conn.session, path, is_dir)
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

fn load_last_log(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().rev().find(|line| !line.trim().is_empty()).map(|line| line.to_string())
}

fn load_log_lines(path: &Path, max_lines: usize) -> VecDeque<String> {
    let mut lines = VecDeque::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return lines;
    };
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        lines.push_back(line.to_string());
        if lines.len() > max_lines {
            lines.pop_front();
        }
    }
    lines
}

fn prune_log_file(path: &Path, days: i64, max_entries: usize) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let cutoff = chrono::Local::now().naive_local() - chrono::Duration::days(days);
    let mut kept = Vec::new();
    for line in content.lines() {
        if let Some((ts, _)) = line.split_once(" | ") {
            if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
                if parsed >= cutoff {
                    kept.push(line.to_string());
                }
            }
        }
    }
    if kept.len() > max_entries {
        kept = kept.split_off(kept.len().saturating_sub(max_entries));
    }
    if kept.is_empty() {
        let _ = std::fs::remove_file(path);
    } else if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(path, kept.join("\n") + "\n");
    }
}

fn read_dir_entries_filtered(dir: &Path, only_dirs: bool) -> Result<Vec<FileEntry>> {
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

fn compute_local_size(path: &Option<PathBuf>, is_dir: bool) -> Result<u64> {
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
