use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ssh2::Session;

use crate::model::{AuthConfig, ConnectionConfig};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn connect_ssh(config: &ConnectionConfig) -> Result<Session> {
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
        let err =
            last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "connect failed"));
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
                .userauth_pubkey_file(&config.user, None, &path, password.as_deref())
                .context("private key auth")?;
        }
    }

    if !session.authenticated() {
        anyhow::bail!("Authentication failed");
    }

    Ok(session)
}

pub(crate) fn run_ssh_terminal(session: &Session) -> Result<()> {
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

pub(crate) fn transfer_path(
    session: &Session,
    source: &Path,
    target_dir: &str,
    source_is_dir: bool,
) -> Result<()> {
    let sftp = session.sftp().context("open sftp")?;
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("missing source filename"))?;
    let remote_base = format!("{}/{}", target_dir.trim_end_matches('/'), name);
    if source_is_dir {
        upload_dir(&sftp, source, &remote_base)?;
    } else {
        upload_file(&sftp, source, &remote_base)?;
    }
    Ok(())
}

fn upload_dir(sftp: &ssh2::Sftp, local_dir: &Path, remote_dir: &str) -> Result<()> {
    let _ = sftp.mkdir(Path::new(remote_dir), 0o755);
    for entry in std::fs::read_dir(local_dir).context("read local dir")? {
        let entry = entry.context("read local dir entry")?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let remote_path = format!("{remote_dir}/{name}");
        if path.is_dir() {
            upload_dir(sftp, &path, &remote_path)?;
        } else {
            upload_file(sftp, &path, &remote_path)?;
        }
    }
    Ok(())
}

fn upload_file(sftp: &ssh2::Sftp, local_path: &Path, remote_path: &str) -> Result<()> {
    let mut local = File::open(local_path).context("open local file")?;
    let mut remote = sftp
        .open_mode(
            Path::new(remote_path),
            ssh2::OpenFlags::CREATE | ssh2::OpenFlags::TRUNCATE | ssh2::OpenFlags::WRITE,
            0o644,
            ssh2::OpenType::File,
        )
        .context("open remote file")?;
    io::copy(&mut local, &mut remote).context("copy file")?;
    Ok(())
}

pub(crate) fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
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
