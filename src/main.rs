use std::fs;
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as Base64;
use base64::Engine;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
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
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConnectionConfig {
    user: String,
    host: String,
    auth: AuthConfig,
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

struct OpenConnection {
    config: ConnectionConfig,
    session: Session,
    connected_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    NewConnection,
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
    status: String,
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
            status: "n = new, c = connect, d = disconnect, q = quit".to_string(),
        })
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::NewConnection => self.handle_new_connection_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('n') => {
                self.mode = Mode::NewConnection;
                self.new_connection = NewConnectionState::default();
                self.status = "Fill fields and press Enter to connect".to_string();
            }
            KeyCode::Char('c') => {
                if let Some(config) = self.connections.get(self.selected_saved).cloned() {
                    if let Err(err) = self.connect_and_open(config) {
                        self.status = format!("Connection failed: {err}");
                    }
                } else {
                    self.status = "No saved connection selected".to_string();
                }
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
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
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
            KeyCode::Enter => {
                match self.build_new_config() {
                    Ok(config) => {
                        match self.connect_and_open(config) {
                            Ok(()) => self.mode = Mode::Normal,
                            Err(err) => {
                                self.status = format!("Connection failed: {err}");
                            }
                        }
                    }
                    Err(err) => {
                        self.status = format!("Missing fields: {err}");
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
        })
    }

    fn connect_and_open(&mut self, config: ConnectionConfig) -> Result<()> {
        let session = connect_ssh(&config)?;
        self.open_connections.push(OpenConnection {
            config: config.clone(),
            session,
            connected_at: SystemTime::now(),
        });
        self.selected_tab = self.open_connections.len().saturating_sub(1);
        self.upsert_connection(config.clone());
        self.save_store()?;
        self.status = format!("Connected to {}", config.label());
        Ok(())
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
            AuthKind::PasswordOnly => fields.push(Field::Password),
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
}

enum EditAction {
    Insert(char),
    Backspace,
}

fn connect_ssh(config: &ConnectionConfig) -> Result<Session> {
    let address = format!("{}:22", config.host);
    let tcp = TcpStream::connect(address).context("connect tcp")?;

    let mut session = Session::new().context("create session")?;
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
        let mut salt = [0u8; 16];
        let mut rng = OsRng;
        rng.try_fill_bytes(&mut salt)
            .map_err(|err| anyhow::anyhow!("random salt failed: {err:?}"))?;
        let key = derive_key(&password, &salt);
        let check = encrypt_string("ssh-client-check", &key)?;
        let master = MasterConfig {
            salt_b64: Base64.encode(salt),
            check,
        };
        return Ok((master, key));
    }
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
    })
}

fn draw_ui(frame: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)].as_ref())
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(layout[0]);

    draw_saved_list(frame, app, body[0]);
    draw_open_tabs(frame, app, body[1]);

    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(status, layout[1]);

    if app.mode == Mode::NewConnection {
        draw_new_connection_modal(frame, app);
    }
}

fn draw_saved_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items: Vec<ListItem> = if app.connections.is_empty() {
        vec![ListItem::new("No saved connections")]
    } else {
        app.connections
            .iter()
            .map(|conn| ListItem::new(conn.label()))
            .collect()
    };

    let list = List::new(items)
        .block(Block::default().title("Saved").borders(Borders::ALL))
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

    let titles: Vec<Line> = if app.open_connections.is_empty() {
        vec![Line::from("No sessions")]
    } else {
        app.open_connections
            .iter()
            .map(|conn| Line::from(conn.config.label()))
            .collect()
    };

    let tabs = Tabs::new(titles)
        .block(Block::default().title("Open").borders(Borders::ALL))
        .select(app.selected_tab)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    frame.render_widget(tabs, tabs_area);

    let content = if let Some(conn) = app.open_connections.get(app.selected_tab) {
        let lines = vec![
            Line::from(vec![
                Span::styled("User: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&conn.config.user),
            ]),
            Line::from(vec![
                Span::styled("Host: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&conn.config.host),
            ]),
            Line::from(vec![
                Span::styled("Auth: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(match &conn.config.auth {
                    AuthConfig::Password { .. } => "Password",
                    AuthConfig::PrivateKey { password: None, .. } => "Private key",
                    AuthConfig::PrivateKey {
                        password: Some(_), ..
                    } => "Private key + password",
                }),
            ]),
            Line::from(vec![
                Span::styled("Connected: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("{:?}", conn.connected_at)),
            ]),
        ];
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: true })
    } else {
        Paragraph::new("No active connection")
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .alignment(Alignment::Center)
    };

    frame.render_widget(content, body_area);
}

fn draw_new_connection_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("New connection")
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

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

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to move, "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to connect, "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to cancel"),
    ]));

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn field_line(label: &str, value: &str, active: bool, mask: bool) -> Line<'static> {
    let display = if mask && !value.is_empty() {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let mut spans = vec![
        Span::styled(
            format!("{label}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(display),
    ];
    if active {
        spans.push(Span::styled("  <", Style::default().fg(Color::Yellow)));
    }
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
