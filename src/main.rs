use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::net::ToSocketAddrs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as Base64;
use base64::Engine;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use pbkdf2::pbkdf2_hmac;
use rand_core::OsRng;
use rand_core::TryRngCore;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Terminal;
use ratatui::{backend::CrosstermBackend, Frame};
use rpassword::prompt_password;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use ssh2::Session;

const TICK_RATE: Duration = Duration::from_millis(150);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HELP_TEXT: &str =
    "n = new | e = edit | c = connect | d = disconnect | t = terminal | m = master pw | x = delete | q = quit";

fn main() -> Result<()> {
    let mut app = App::load_with_master()?;

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut last_tick = std::time::Instant::now();

    loop {
        terminal.draw(|frame| draw_ui(frame, &app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if app.handle_key(key)? {
                    return Ok(());
                }
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = std::time::Instant::now();
        }

        if let Some(action) = app.pending_action.take() {
            match action {
                AppAction::OpenTerminal => {
                    handle_terminal_mode(terminal, app)?;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConnectionConfig {
    user: String,
    host: String,
    auth: AuthConfig,
    #[serde(default)]
    history: Vec<HistoryEntry>,
}

impl ConnectionConfig {
    fn label(&self) -> String {
        let auth_label = match &self.auth {
            AuthConfig::Password { .. } => "pw",
            AuthConfig::PrivateKey { password: None, .. } => "pk",
            AuthConfig::PrivateKey {
                password: Some(_), ..
            } => "pk+pw",
        };
        format!("{}@{} ({})", self.user, self.host, auth_label)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum AuthConfig {
    Password { password: String },
    PrivateKey { path: String, password: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum HistoryState {
    Success,
    Failure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HistoryEntry {
    ts: u64,
    state: HistoryState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreFile {
    master: MasterConfig,
    connections: Vec<StoredConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MasterConfig {
    salt_b64: String,
    check: EncryptedBlob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedBlob {
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredConnection {
    user: String,
    host: String,
    auth: StoredAuthConfig,
    #[serde(default, deserialize_with = "deserialize_history")]
    history: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum StoredAuthConfig {
    Password { password: EncryptedBlob },
    PrivateKey { path: String, password: Option<EncryptedBlob> },
}

fn same_identity(left: &ConnectionConfig, right: &ConnectionConfig) -> bool {
    if left.user != right.user || left.host != right.host {
        return false;
    }
    match (&left.auth, &right.auth) {
        (AuthConfig::Password { .. }, AuthConfig::Password { .. }) => true,
        (
            AuthConfig::PrivateKey { path: left_path, .. },
            AuthConfig::PrivateKey { path: right_path, .. },
        ) => left_path == right_path,
        _ => false,
    }
}

fn connection_key(conn: &ConnectionConfig) -> String {
    let auth_key = match &conn.auth {
        AuthConfig::Password { .. } => "pw".to_string(),
        AuthConfig::PrivateKey { path, .. } => format!("pk:{}", path),
    };
    format!("{}@{}|{}", conn.user, conn.host, auth_key)
}

struct OpenConnection {
    config: ConnectionConfig,
    session: Session,
    connected_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    NewConnection,
    ChangeMasterPassword,
    ConfirmDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    User,
    Host,
    AuthType,
    KeyPath,
    Password,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MasterField {
    Current,
    New,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthKind {
    PasswordOnly,
    PrivateKey,
    PrivateKeyWithPassword,
}

#[derive(Debug, Clone)]
struct NewConnectionState {
    user: String,
    host: String,
    auth_kind: AuthKind,
    key_path: String,
    password: String,
    active_field: Field,
}

impl Default for NewConnectionState {
    fn default() -> Self {
        Self {
            user: String::new(),
            host: String::new(),
            auth_kind: AuthKind::PasswordOnly,
            key_path: String::new(),
            password: String::new(),
            active_field: Field::User,
        }
    }
}

#[derive(Debug, Clone)]
struct MasterPasswordState {
    current: String,
    new_password: String,
    confirm: String,
    active_field: MasterField,
}

impl Default for MasterPasswordState {
    fn default() -> Self {
        Self {
            current: String::new(),
            new_password: String::new(),
            confirm: String::new(),
            active_field: MasterField::Current,
        }
    }
}
#[derive(Debug, Clone)]
struct FileEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Debug, Clone)]
struct FilePickerState {
    cwd: PathBuf,
    entries: Vec<FileEntry>,
    selected: usize,
}

#[derive(Debug, Clone)]
struct TryResult {
    success: bool,
    message: String,
}

#[derive(Debug, Clone)]
enum AppAction {
    OpenTerminal,
}

struct App {
    config_path: PathBuf,
    master: MasterConfig,
    master_key: Vec<u8>,
    connections: Vec<ConnectionConfig>,
    selected_saved: usize,
    selected_tab: usize,
    open_connections: Vec<OpenConnection>,
    mode: Mode,
    new_connection: NewConnectionState,
    master_change: MasterPasswordState,
    status: String,
    file_picker: Option<FilePickerState>,
    pending_action: Option<AppAction>,
    last_error: HashMap<String, String>,
    edit_index: Option<usize>,
    delete_index: Option<usize>,
    try_result: Option<TryResult>,
    new_connection_feedback: Option<String>,
}

impl App {
    fn load_with_master() -> Result<Self> {
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
            pending_action: None,
            last_error: HashMap::new(),
            edit_index: None,
            delete_index: None,
            try_result: None,
            new_connection_feedback: None,
        })
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::NewConnection => self.handle_new_connection_key(key),
            Mode::ChangeMasterPassword => self.handle_master_password_key(key),
            Mode::ConfirmDelete => self.handle_confirm_delete_key(key),
        }
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
            KeyCode::Char('t') => {
                if self.open_connections.is_empty() {
                    self.status = "No open connections".to_string();
                } else {
                    self.pending_action = Some(AppAction::OpenTerminal);
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
            KeyCode::Up => {
                if self.selected_saved > 0 {
                    self.selected_saved -= 1;
                }
            }
            KeyCode::Down => {
                if self.selected_saved + 1 < self.connections.len() {
                    self.selected_saved += 1;
                }
            }
            KeyCode::Left => {
                if self.selected_tab > 0 {
                    self.selected_tab -= 1;
                }
            }
            KeyCode::Right => {
                if self.selected_tab + 1 < self.open_connections.len() {
                    self.selected_tab += 1;
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
            KeyCode::Tab => self.advance_field(true),
            KeyCode::BackTab => self.advance_field(false),
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
            KeyCode::Char('t') => {
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
            KeyCode::Enter => {
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
                        self.last_error.remove(&connection_key(&removed));
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

    fn connect_and_open(&mut self, config: ConnectionConfig) -> Result<()> {
        let mut config = config;
        let session = connect_ssh(&config)?;
        config.history.push(HistoryEntry {
            ts: now_epoch(),
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
        self.last_error.remove(&connection_key(&config));
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
        if let Some(pos) = fields.iter().position(|field| *field == self.new_connection.active_field) {
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
        fields
    }

    fn edit_active_field(&mut self, action: EditAction) {
        let target = match self.new_connection.active_field {
            Field::User => &mut self.new_connection.user,
            Field::Host => &mut self.new_connection.host,
            Field::KeyPath => &mut self.new_connection.key_path,
            Field::Password => &mut self.new_connection.password,
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
        let current_key = derive_key(&self.master_change.current, &salt);
        let check = decrypt_string(&self.master.check, &current_key)
            .context("verify current password")?;
        if check != "ssh-client-check" {
            anyhow::bail!("Current master password incorrect");
        }

        let (new_master, new_key) = create_master_from_password(&self.master_change.new_password)?;
        let stored = StoreFile {
            master: new_master.clone(),
            connections: self
                .connections
                .iter()
                .map(|conn| encrypt_connection(conn, &new_key))
                .collect::<Result<Vec<_>>>()?,
        };
        save_store(&self.config_path, &stored)?;
        self.master = new_master;
        self.master_key = new_key;
        self.master_change = MasterPasswordState::default();
        Ok(())
    }

    fn record_connect_error(&mut self, config: &ConnectionConfig, err: &anyhow::Error) {
        self.last_error
            .insert(connection_key(config), format!("{err}"));
        let mut should_save = false;
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|conn| same_identity(conn, config))
        {
            existing.history.push(HistoryEntry {
                ts: now_epoch(),
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

    fn upsert_connection(&mut self, connection: ConnectionConfig) {
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|c| same_identity(c, &connection))
        {
            *existing = connection;
            return;
        }
        self.connections.push(connection);
    }

    fn save_store(&self) -> Result<()> {
        let stored = StoreFile {
            master: self.master.clone(),
            connections: self
                .connections
                .iter()
                .map(|conn| encrypt_connection(conn, &self.master_key))
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

    fn handle_file_picker_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(picker) = &mut self.file_picker {
            match key.code {
                KeyCode::Esc => {
                    self.file_picker = None;
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
                            picker.cwd = entry.path;
                            picker.entries = read_dir_entries(&picker.cwd)?;
                            picker.selected = 0;
                        } else {
                            self.new_connection.key_path =
                                entry.path.to_string_lossy().into_owned();
                            self.file_picker = None;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }
}

enum EditAction {
    Insert(char),
    Backspace,
}

fn connect_ssh(config: &ConnectionConfig) -> Result<Session> {
    let address = format!("{}:22", config.host);
    let mut last_err = None;
    let mut tcp = None;
    for addr in address
        .to_socket_addrs()
        .context("resolve address")?
    {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(err) => last_err = Some(err),
        }
    }
    let tcp = tcp.ok_or_else(|| {
        let err = last_err.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "connect failed")
        });
        anyhow::anyhow!("connect tcp failed: {err}")
    })?;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT)).ok();
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).ok();

    let mut session = Session::new().context("create session")?;
    session.set_timeout(CONNECT_TIMEOUT.as_millis() as u32);
    session.set_tcp_stream(tcp);
    session.handshake().context("ssh handshake")?;

    match &config.auth {
        AuthConfig::Password { password } => {
            session
                .userauth_password(&config.user, password)
                .context("password auth")?;
        }
        AuthConfig::PrivateKey { path, password } => {
            let path = expand_tilde(path);
            if !path.exists() {
                anyhow::bail!("Private key not found at {}", path.display());
            }
            session
                .userauth_pubkey_file(
                    &config.user,
                    None,
                    &path,
                    password.as_deref(),
                )
                .context("private key auth")?;
        }
    }

    if !session.authenticated() {
        anyhow::bail!("Authentication failed");
    }

    Ok(session)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

fn config_path() -> Result<PathBuf> {
    if let Some(mut dir) = dirs::config_dir() {
        dir.push("ssh-client");
        dir.push("config.json");
        return Ok(dir);
    }
    let mut fallback = std::env::current_dir().context("current dir")?;
    fallback.push("ssh-client-config.json");
    Ok(fallback)
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
        let name = entry
            .file_name()
            .to_string_lossy()
            .into_owned();
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

fn handle_master_password_change(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

    let result = change_master_password(app);

    execute!(terminal.backend_mut(), EnterAlternateScreen).ok();
    enable_raw_mode().ok();
    terminal.clear().ok();

    match result {
        Ok(()) => app.status = "Master password updated".to_string(),
        Err(err) => app.status = format!("Master password not changed: {err}"),
    }

    Ok(())
}

fn change_master_password(app: &mut App) -> Result<()> {
    let current = prompt_password("Current master password: ").context("read current password")?;
    let salt = Base64.decode(&app.master.salt_b64).context("decode salt")?;
    let current_key = derive_key(&current, &salt);
    let check = decrypt_string(&app.master.check, &current_key).context("verify current password")?;
    if check != "ssh-client-check" {
        anyhow::bail!("Current master password incorrect");
    }

    let (new_master, new_key) = setup_master()?;
    let stored = StoreFile {
        master: new_master.clone(),
        connections: app
            .connections
            .iter()
            .map(|conn| encrypt_connection(conn, &new_key))
            .collect::<Result<Vec<_>>>()?,
    };

    save_store(&app.config_path, &stored)?;
    app.master = new_master;
    app.master_key = new_key;
    Ok(())
}

fn handle_terminal_mode(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let session = match app.open_connections.get(app.selected_tab) {
        Some(conn) => &conn.session,
        None => {
            app.status = "No active connection".to_string();
            return Ok(());
        }
    };

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::terminal::Clear(ClearType::All)
    )
    .ok();

    let result = run_ssh_terminal(session);

    execute!(terminal.backend_mut(), EnterAlternateScreen).ok();
    enable_raw_mode().ok();
    terminal.clear().ok();

    match result {
        Ok(()) => app.status = "Exited terminal session".to_string(),
        Err(err) => app.status = format!("Terminal session error: {err}"),
    }

    Ok(())
}

fn run_ssh_terminal(session: &Session) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut channel = session.channel_session().context("open channel")?;
    channel
        .request_pty("xterm", None, Some((u32::from(cols), u32::from(rows), 0, 0)))
        .context("request pty")?;
    channel.shell().context("start shell")?;
    session.set_blocking(false);

    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Connected. Press Ctrl+g to return to the client."
    )
    .ok();
    stdout.flush().ok();

    let mut buffer = [0u8; 4096];
    let mut err_buffer = [0u8; 1024];

    loop {
        if channel.eof() {
            break;
        }

        match channel.read(&mut buffer) {
            Ok(0) => {}
            Ok(count) => {
                stdout.write_all(&buffer[..count]).ok();
                stdout.flush().ok();
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err).context("read channel");
                }
            }
        }

        match channel.stderr().read(&mut err_buffer) {
            Ok(0) => {}
            Ok(count) => {
                stdout.write_all(&err_buffer[..count]).ok();
                stdout.flush().ok();
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err).context("read stderr");
                }
            }
        }

        if event::poll(Duration::from_millis(30))? {
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('g'))
                {
                    break;
                }
                if let Some(bytes) = key_to_bytes(key) {
                    channel.write_all(&bytes).ok();
                    channel.flush().ok();
                }
            }
        }
    }

    session.set_blocking(true);
    channel.close().ok();
    Ok(())
}

fn key_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let byte = (c as u8) & 0x1f;
                Some(vec![byte])
            } else {
                Some(c.to_string().into_bytes())
            }
        }
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        _ => None,
    }
}

fn load_or_init_store(path: &Path) -> Result<(MasterConfig, Vec<u8>, Vec<ConnectionConfig>)> {
    if path.exists() {
        let store = load_store(path)?;
        let master_key = prompt_existing_master(&store.master)?;
        let connections = store
            .connections
            .into_iter()
            .map(|conn| decrypt_connection(conn, &master_key))
            .collect::<Result<Vec<_>>>()?;
        return Ok((store.master, master_key, connections));
    }

    let (master, master_key) = setup_master()?;
    let store = StoreFile {
        master: master.clone(),
        connections: vec![],
    };
    save_store(path, &store)?;
    Ok((master, master_key, vec![]))
}

fn load_store(path: &Path) -> Result<StoreFile> {
    let content = fs::read_to_string(path).context("read config file")?;
    let store = serde_json::from_str(&content).context("parse config file")?;
    Ok(store)
}

fn save_store(path: &Path, store: &StoreFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create config dir")?;
    }
    let content = serde_json::to_string_pretty(store).context("serialize config")?;
    fs::write(path, content).context("write config file")?;
    Ok(())
}

fn prompt_existing_master(master: &MasterConfig) -> Result<Vec<u8>> {
    loop {
        let password = prompt_password("Master password: ").context("read master password")?;
        let salt = Base64.decode(&master.salt_b64).context("decode salt")?;
        let key = derive_key(&password, &salt);
        match decrypt_string(&master.check, &key) {
            Ok(check) if check == "ssh-client-check" => return Ok(key),
            _ => {
                eprintln!("Invalid master password.");
            }
        }
    }
}

fn setup_master() -> Result<(MasterConfig, Vec<u8>)> {
    loop {
        let password = prompt_password("Set master password: ").context("read master password")?;
        let confirm = prompt_password("Confirm master password: ").context("read confirm password")?;
        if password != confirm {
            eprintln!("Passwords do not match.");
            continue;
        }
        if password.is_empty() {
            eprintln!("Master password cannot be empty.");
            continue;
        }
        return create_master_from_password(&password);
    }
}

fn create_master_from_password(password: &str) -> Result<(MasterConfig, Vec<u8>)> {
    let mut salt = [0u8; 16];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut salt)
        .map_err(|err| anyhow::anyhow!("random salt failed: {err:?}"))?;
    let key = derive_key(password, &salt);
    let check = encrypt_string("ssh-client-check", &key)?;
    let master = MasterConfig {
        salt_b64: Base64.encode(salt),
        check,
    };
    Ok((master, key))
}

fn derive_key(password: &str, salt: &[u8]) -> Vec<u8> {
    let mut key = vec![0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 100_000, &mut key);
    key
}

fn encrypt_string(plaintext: &str, key: &[u8]) -> Result<EncryptedBlob> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut nonce_bytes)
        .map_err(|err| anyhow::anyhow!("random nonce failed: {err:?}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|err| anyhow::anyhow!("encrypt failed: {err:?}"))?;
    Ok(EncryptedBlob {
        nonce: Base64.encode(nonce_bytes),
        ciphertext: Base64.encode(ciphertext),
    })
}

fn decrypt_string(blob: &EncryptedBlob, key: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes = Base64.decode(&blob.nonce).context("decode nonce")?;
    let ciphertext = Base64
        .decode(&blob.ciphertext)
        .context("decode ciphertext")?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|err| anyhow::anyhow!("decrypt failed: {err:?}"))?;
    let text = String::from_utf8(plaintext).context("decode utf8")?;
    Ok(text)
}

fn encrypt_connection(conn: &ConnectionConfig, key: &[u8]) -> Result<StoredConnection> {
    let auth = match &conn.auth {
        AuthConfig::Password { password } => StoredAuthConfig::Password {
            password: encrypt_string(password, key)?,
        },
        AuthConfig::PrivateKey { path, password } => StoredAuthConfig::PrivateKey {
            path: path.clone(),
            password: match password {
                Some(pass) => Some(encrypt_string(pass, key)?),
                None => None,
            },
        },
    };
    Ok(StoredConnection {
        user: conn.user.clone(),
        host: conn.host.clone(),
        auth,
        history: conn.history.clone(),
    })
}

fn decrypt_connection(conn: StoredConnection, key: &[u8]) -> Result<ConnectionConfig> {
    let auth = match conn.auth {
        StoredAuthConfig::Password { password } => AuthConfig::Password {
            password: decrypt_string(&password, key)?,
        },
        StoredAuthConfig::PrivateKey { path, password } => AuthConfig::PrivateKey {
            path,
            password: match password {
                Some(pass) => Some(decrypt_string(&pass, key)?),
                None => None,
            },
        },
    };
    Ok(ConnectionConfig {
        user: conn.user,
        host: conn.host,
        auth,
        history: conn.history,
    })
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn format_history_entry(entry: &HistoryEntry) -> String {
    let dt = chrono::DateTime::<Local>::from(
        SystemTime::UNIX_EPOCH + Duration::from_secs(entry.ts),
    );
    let state = match entry.state {
        HistoryState::Success => "success",
        HistoryState::Failure => "failed",
    };
    format!("{} | {}", dt.format("%Y-%m-%d %H:%M:%S"), state)
}

fn deserialize_history<'de, D>(deserializer: D) -> Result<Vec<HistoryEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HistoryCompat {
        Entries(Vec<HistoryEntry>),
        Timestamps(Vec<u64>),
    }

    match HistoryCompat::deserialize(deserializer)? {
        HistoryCompat::Entries(entries) => Ok(entries),
        HistoryCompat::Timestamps(timestamps) => Ok(timestamps
            .into_iter()
            .map(|ts| HistoryEntry {
                ts,
                state: HistoryState::Success,
            })
            .collect()),
    }
}

fn draw_ui(frame: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1)].as_ref())
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(layout[0]);

    draw_saved_list(frame, app, body[0]);
    draw_open_tabs(frame, app, body[1]);

    if app.mode == Mode::NewConnection {
        draw_new_connection_modal(frame, app);
        if app.file_picker.is_some() {
            draw_file_picker_modal(frame, app);
        }
        if app.try_result.is_some() {
            draw_try_result_modal(frame, app);
        }
    }
    if app.mode == Mode::ChangeMasterPassword {
        draw_master_password_modal(frame, app);
    }
    if app.mode == Mode::ConfirmDelete {
        draw_confirm_delete_modal(frame, app);
    }
}

fn draw_saved_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let connected: HashSet<String> = app
        .open_connections
        .iter()
        .map(|conn| connection_key(&conn.config))
        .collect();
    let items: Vec<ListItem> = if app.connections.is_empty() {
        vec![ListItem::new("No saved connections")]
    } else {
        app.connections
            .iter()
            .map(|conn| {
                let key = connection_key(conn);
                let status_style = if connected.contains(&key) {
                    Style::default().fg(Color::Green)
                } else if app.last_error.contains_key(&key) {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                let prefix = if connected.contains(&key) {
                    "C "
                } else if app.last_error.contains_key(&key) {
                    "! "
                } else {
                    "  "
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{prefix}{}", conn.label()),
                    status_style,
                )))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title("Available connections")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    frame.render_stateful_widget(
        list,
        area,
        &mut list_state(app.selected_saved, app.connections.len()),
    );
}

fn draw_open_tabs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let tabs_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 3,
    };
    let body_area = Rect {
        x: area.x,
        y: area.y + 3,
        width: area.width,
        height: area.height.saturating_sub(3),
    };
    let help = Paragraph::new(HELP_TEXT)
        .block(Block::default().title("Help").borders(Borders::ALL))
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(help, tabs_area);

    let connected: HashSet<String> = app
        .open_connections
        .iter()
        .map(|conn| connection_key(&conn.config))
        .collect();
    let content = if let Some(conn) = app.connections.get(app.selected_saved) {
        let key = connection_key(conn);
        let status = if connected.contains(&key) {
            "Connected"
        } else if app.last_error.contains_key(&key) {
            "Failed"
        } else if conn.history.is_empty() {
            "Never connected"
        } else {
            "Not connected"
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled("User: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&conn.user),
            ]),
            Line::from(vec![
                Span::styled("Host: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&conn.host),
            ]),
            Line::from(vec![
                Span::styled("Auth: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(match &conn.auth {
                    AuthConfig::Password { .. } => "Password",
                    AuthConfig::PrivateKey { password: None, .. } => "Private key",
                    AuthConfig::PrivateKey {
                        password: Some(_), ..
                    } => "Private key + password",
                }),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(status),
            ]),
        ];

        if let Some(err) = app.last_error.get(&key) {
            lines.push(Line::from(vec![
                Span::styled("Error: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(err, Style::default().fg(Color::Red)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Past connections:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if conn.history.is_empty() {
            lines.push(Line::from("  (none)"));
        } else {
            for entry in conn.history.iter().rev().take(5) {
                lines.push(Line::from(format!(
                    "  {}",
                    format_history_entry(entry)
                )));
            }
        }

        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Connection details"),
            )
            .wrap(Wrap { trim: true })
    } else {
        Paragraph::new("No saved connection selected")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Connection details"),
            )
            .alignment(Alignment::Center)
    };

    frame.render_widget(content, body_area);
}

fn draw_new_connection_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let title = if app.edit_index.is_some() {
        "Edit connection"
    } else {
        "New connection"
    };
    let block = Block::default().title(title).borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)].as_ref())
        .split(inner);

    let mut lines = Vec::new();
    lines.push(field_line(
        "User",
        &app.new_connection.user,
        app.new_connection.active_field == Field::User,
        false,
    ));
    lines.push(field_line(
        "Host",
        &app.new_connection.host,
        app.new_connection.active_field == Field::Host,
        false,
    ));
    lines.push(field_line(
        "Auth",
        auth_kind_label(app.new_connection.auth_kind),
        app.new_connection.active_field == Field::AuthType,
        false,
    ));
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PrivateKey | AuthKind::PrivateKeyWithPassword
    ) {
        lines.push(field_line(
            "Key path",
            &app.new_connection.key_path,
            app.new_connection.active_field == Field::KeyPath,
            false,
        ));
        lines.push(Line::from(Span::styled(
            "F2 to browse for key file",
            Style::default().fg(Color::Gray),
        )));
    }
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PasswordOnly | AuthKind::PrivateKeyWithPassword
    ) {
        lines.push(field_line(
            "Password",
            &app.new_connection.password,
            app.new_connection.active_field == Field::Password,
            true,
        ));
    }

    if let Some(message) = &app.new_connection_feedback {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, layout[0]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to move, "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(if app.edit_index.is_some() {
            " to save, "
        } else {
            " to connect, "
        }),
        Span::styled("T", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to try, "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to cancel"),
    ]))
    .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn draw_file_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.file_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Pick key file")
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3), Constraint::Length(2)].as_ref())
        .split(inner);

    let header = Paragraph::new(format!("Dir: {}", picker.cwd.display()))
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(header, layout[0]);

    let items: Vec<ListItem> = if picker.entries.is_empty() {
        vec![ListItem::new("Empty")]
    } else {
        picker
            .entries
            .iter()
            .map(|entry| {
                let prefix = if entry.is_dir { "[D]" } else { "[F]" };
                ListItem::new(format!("{prefix} {}", entry.name))
            })
            .collect()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    frame.render_stateful_widget(
        list,
        layout[1],
        &mut list_state(picker.selected, picker.entries.len()),
    );

    let footer = Paragraph::new("Enter to open/select, Backspace to up, Esc to cancel")
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[2]);
}

fn draw_master_password_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(60, 45, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Change master password")
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let lines = vec![
        field_line(
            "Current",
            &app.master_change.current,
            app.master_change.active_field == MasterField::Current,
            true,
        ),
        field_line(
            "New",
            &app.master_change.new_password,
            app.master_change.active_field == MasterField::New,
            true,
        ),
        field_line(
            "Confirm",
            &app.master_change.confirm,
            app.master_change.active_field == MasterField::Confirm,
            true,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to move, "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to save, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn draw_confirm_delete_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(50, 30, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Delete connection?")
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let label = app
        .delete_index
        .and_then(|index| app.connections.get(index))
        .map(|conn| conn.label())
        .unwrap_or_else(|| "Unknown".to_string());

    let lines = vec![
        Line::from(format!("Delete {label}?")),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" or "),
            Span::styled("Y", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to confirm, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" or "),
            Span::styled("N", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn draw_try_result_modal(frame: &mut Frame<'_>, app: &App) {
    let result = match &app.try_result {
        Some(result) => result,
        None => return,
    };
    let area = centered_rect(50, 25, frame.area());
    frame.render_widget(Clear, area);
    let title = if result.success { "Try success" } else { "Try failed" };
    let block = Block::default().title(title).borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(2)].as_ref())
        .split(inner);

    let message = Paragraph::new(result.message.as_str()).wrap(Wrap { trim: true });
    frame.render_widget(message, layout[0]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" or "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to close"),
    ]))
    .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn field_line(label: &str, value: &str, active: bool, mask: bool) -> Line<'static> {
    let display = if mask && !value.is_empty() {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let indicator = if active { "> " } else { "  " };
    let spans = vec![
        Span::styled(indicator, Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{label}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(display),
    ];
    Line::from(spans)
}

fn auth_kind_label(kind: AuthKind) -> &'static str {
    match kind {
        AuthKind::PasswordOnly => "Password only",
        AuthKind::PrivateKey => "Private key",
        AuthKind::PrivateKeyWithPassword => "Private key + password",
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

fn list_state(selected: usize, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(selected.min(len.saturating_sub(1))));
    }
    state
}
