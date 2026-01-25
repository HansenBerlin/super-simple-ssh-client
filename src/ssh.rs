use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
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
    for addr in address.to_socket_addrs().context("resolve address")? {
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
        .request_pty(
            "xterm",
            None,
            Some((u32::from(cols), u32::from(rows), 0, 0)),
        )
        .context("request pty")?;
    channel.shell().context("start shell")?;
    session.set_blocking(false);

    let mut stdout = io::stdout();
    writeln!(stdout, "Connected. Press Ctrl+g to return to the client.").ok();
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

pub(crate) fn remote_size(session: &Session, path: &str, is_dir: bool) -> Result<u64> {
    let sftp = session.sftp().context("open sftp")?;
    if !is_dir {
        let stat = sftp.stat(Path::new(path)).context("stat remote file")?;
        return Ok(stat.size.unwrap_or(0));
    }
    fn walk(sftp: &ssh2::Sftp, path: &str) -> Result<u64> {
        let mut total = 0u64;
        for (child, stat) in sftp.readdir(Path::new(path)).context("read remote dir")? {
            let name = child
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| String::from("/"));
            if name == "." || name == ".." {
                continue;
            }
            let child_path = if path.ends_with('/') {
                format!("{path}{name}")
            } else {
                format!("{path}/{name}")
            };
            let is_dir = stat.perm.unwrap_or(0) & 0o040000 != 0;
            if is_dir {
                total = total.saturating_add(walk(sftp, &child_path)?);
            } else {
                total = total.saturating_add(stat.size.unwrap_or(0));
            }
        }
        Ok(total)
    }
    walk(&sftp, path)
}

pub(crate) fn remote_has_subdirectories(session: &Session, path: &str) -> Result<bool> {
    let sftp = session.sftp().context("open sftp")?;
    for (_child, stat) in sftp.readdir(Path::new(path)).context("read remote dir")? {
        let is_dir = stat.perm.unwrap_or(0) & 0o040000 != 0;
        if is_dir {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn transfer_path_with_progress(
    session: &Session,
    source: &Path,
    target_dir: &str,
    source_is_dir: bool,
    tx: &std::sync::mpsc::Sender<crate::model::TransferUpdate>,
    cancel_rx: &std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    let sftp = session.sftp().context("open sftp")?;
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("missing source filename"))?;
    let remote_base = format!("{}/{}", target_dir.trim_end_matches('/'), name);
    if source_is_dir {
        upload_dir_with_progress(&sftp, source, &remote_base, tx, cancel_rx)?;
    } else {
        upload_file_with_progress(&sftp, source, &remote_base, tx, cancel_rx)?;
    }
    Ok(())
}

pub(crate) fn download_path_with_progress(
    session: &Session,
    source: &str,
    target_dir: &Path,
    source_is_dir: bool,
    tx: &std::sync::mpsc::Sender<crate::model::TransferUpdate>,
    cancel_rx: &std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    let sftp = session.sftp().context("open sftp")?;
    let name = Path::new(source)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("missing source filename"))?;
    let local_base = target_dir.join(name);
    if source_is_dir {
        download_dir_with_progress(&sftp, source, &local_base, tx, cancel_rx)?;
    } else {
        download_file_with_progress(&sftp, source, &local_base, tx, cancel_rx)?;
    }
    Ok(())
}

fn upload_dir_with_progress(
    sftp: &ssh2::Sftp,
    local_dir: &Path,
    remote_dir: &str,
    tx: &std::sync::mpsc::Sender<crate::model::TransferUpdate>,
    cancel_rx: &std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    check_cancel(cancel_rx)?;
    let _ = sftp.mkdir(Path::new(remote_dir), 0o755);
    for entry in std::fs::read_dir(local_dir).context("read local dir")? {
        let entry = entry.context("read local dir entry")?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let remote_path = format!("{remote_dir}/{name}");
        if path.is_dir() {
            upload_dir_with_progress(sftp, &path, &remote_path, tx, cancel_rx)?;
        } else {
            upload_file_with_progress(sftp, &path, &remote_path, tx, cancel_rx)?;
        }
    }
    Ok(())
}

fn upload_file_with_progress(
    sftp: &ssh2::Sftp,
    local_path: &Path,
    remote_path: &str,
    tx: &std::sync::mpsc::Sender<crate::model::TransferUpdate>,
    cancel_rx: &std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    let mut local = File::open(local_path).context("open local file")?;
    let mut remote = sftp
        .open_mode(
            Path::new(remote_path),
            ssh2::OpenFlags::CREATE | ssh2::OpenFlags::TRUNCATE | ssh2::OpenFlags::WRITE,
            0o644,
            ssh2::OpenType::File,
        )
        .context("open remote file")?;
    let mut buffer = [0u8; 8192];
    loop {
        check_cancel(cancel_rx)?;
        let read = local.read(&mut buffer).context("read local file")?;
        if read == 0 {
            break;
        }
        remote
            .write_all(&buffer[..read])
            .context("write remote file")?;
        let _ = tx.send(crate::model::TransferUpdate::Bytes(read as u64));
    }
    Ok(())
}

fn download_dir_with_progress(
    sftp: &ssh2::Sftp,
    remote_dir: &str,
    local_dir: &Path,
    tx: &std::sync::mpsc::Sender<crate::model::TransferUpdate>,
    cancel_rx: &std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    check_cancel(cancel_rx)?;
    std::fs::create_dir_all(local_dir).context("create local dir")?;
    for (path, stat) in sftp
        .readdir(Path::new(remote_dir))
        .context("read remote dir")?
    {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("/"));
        if name == "." || name == ".." {
            continue;
        }
        let is_dir = stat.perm.unwrap_or(0) & 0o040000 != 0;
        let remote_path = if remote_dir.ends_with('/') {
            format!("{remote_dir}{name}")
        } else {
            format!("{remote_dir}/{name}")
        };
        let local_path = local_dir.join(&name);
        if is_dir {
            download_dir_with_progress(sftp, &remote_path, &local_path, tx, cancel_rx)?;
        } else {
            download_file_with_progress(sftp, &remote_path, &local_path, tx, cancel_rx)?;
        }
    }
    Ok(())
}

fn download_file_with_progress(
    sftp: &ssh2::Sftp,
    remote_path: &str,
    local_path: &Path,
    tx: &std::sync::mpsc::Sender<crate::model::TransferUpdate>,
    cancel_rx: &std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    let mut remote = sftp
        .open(Path::new(remote_path))
        .context("open remote file")?;
    let mut local = File::create(local_path).context("create local file")?;
    let mut buffer = [0u8; 8192];
    loop {
        check_cancel(cancel_rx)?;
        let read = remote.read(&mut buffer).context("read remote file")?;
        if read == 0 {
            break;
        }
        local
            .write_all(&buffer[..read])
            .context("write local file")?;
        let _ = tx.send(crate::model::TransferUpdate::Bytes(read as u64));
    }
    Ok(())
}

fn check_cancel(cancel_rx: &std::sync::mpsc::Receiver<()>) -> Result<()> {
    if cancel_rx.try_recv().is_ok() {
        anyhow::bail!("Transfer cancelled");
    }
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
