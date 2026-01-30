use std::path::Path;

use anyhow::{Context, Result};

use crate::model::{ConnectionConfig, OpenConnection, RemoteEntry};
use crate::ssh::connect_ssh;

#[cfg(test)]
use std::collections::HashMap;

pub(crate) trait SshBackend: Send + Sync {
    fn list_remote_dir(
        &self,
        open_connections: Option<&[OpenConnection]>,
        conn: &ConnectionConfig,
        cwd: &str,
        only_dirs: bool,
        show_hidden: bool,
    ) -> Result<Vec<RemoteEntry>>;
    fn remote_home_dir(
        &self,
        open_connections: Option<&[OpenConnection]>,
        conn: &ConnectionConfig,
    ) -> Result<Option<String>>;
    fn remote_has_subdirectories(
        &self,
        open_connections: Option<&[OpenConnection]>,
        conn: &ConnectionConfig,
        path: &str,
    ) -> Result<bool>;
    fn remote_size(&self, conn: &ConnectionConfig, path: &str, is_dir: bool) -> Result<u64>;
}

#[derive(Debug, Default)]
pub(crate) struct RealSshBackend;

impl RealSshBackend {
    fn find_session<'a>(
        &self,
        open_connections: Option<&'a [OpenConnection]>,
        conn: &ConnectionConfig,
    ) -> Option<&'a ssh2::Session> {
        let open_connections = open_connections?;
        open_connections
            .iter()
            .find(|candidate| crate::model::same_identity(&candidate.config, conn))
            .map(|conn| &conn.session)
    }
}

impl SshBackend for RealSshBackend {
    fn list_remote_dir(
        &self,
        open_connections: Option<&[OpenConnection]>,
        conn: &ConnectionConfig,
        cwd: &str,
        only_dirs: bool,
        show_hidden: bool,
    ) -> Result<Vec<RemoteEntry>> {
        let session = if let Some(session) = self.find_session(open_connections, conn) {
            session
        } else {
            let session = connect_ssh(conn)?;
            return list_remote_dir_with_session(&session, cwd, only_dirs, show_hidden);
        };
        list_remote_dir_with_session(session, cwd, only_dirs, show_hidden)
    }

    fn remote_home_dir(
        &self,
        open_connections: Option<&[OpenConnection]>,
        conn: &ConnectionConfig,
    ) -> Result<Option<String>> {
        if let Some(session) = self.find_session(open_connections, conn) {
            let home = crate::ssh::remote_home_dir(session)?;
            return Ok(if home.trim().is_empty() {
                None
            } else {
                Some(home)
            });
        }
        let session = connect_ssh(conn)?;
        let home = crate::ssh::remote_home_dir(&session)?;
        Ok(if home.trim().is_empty() { None } else { Some(home) })
    }

    fn remote_has_subdirectories(
        &self,
        open_connections: Option<&[OpenConnection]>,
        conn: &ConnectionConfig,
        path: &str,
    ) -> Result<bool> {
        let session = if let Some(session) = self.find_session(open_connections, conn) {
            session
        } else {
            let session = connect_ssh(conn)?;
            return crate::ssh::remote_has_subdirectories(&session, path);
        };
        crate::ssh::remote_has_subdirectories(session, path)
    }

    fn remote_size(&self, conn: &ConnectionConfig, path: &str, is_dir: bool) -> Result<u64> {
        let session = connect_ssh(conn)?;
        crate::ssh::remote_size(&session, path, is_dir)
    }
}

fn list_remote_dir_with_session(
    session: &ssh2::Session,
    cwd: &str,
    only_dirs: bool,
    show_hidden: bool,
) -> Result<Vec<RemoteEntry>> {
    let sftp = session.sftp().context("open sftp")?;
    let mut entries = Vec::new();
    for (path, stat) in sftp.readdir(Path::new(cwd)).context("read remote dir")? {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("/"));
        if name == "." || name == ".." {
            continue;
        }
        if !show_hidden && name.starts_with('.') {
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
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct MockSshBackend {
    list: std::sync::Mutex<HashMap<String, Vec<RemoteEntry>>>,
    list_errors: std::sync::Mutex<HashMap<String, String>>,
    home: std::sync::Mutex<Option<String>>,
    has_subdirs: std::sync::Mutex<bool>,
    size: std::sync::Mutex<u64>,
    size_error: std::sync::Mutex<Option<String>>,
}

#[cfg(test)]
impl MockSshBackend {
    pub(crate) fn set_list(&self, path: &str, result: Result<Vec<RemoteEntry>>) {
        match result {
            Ok(entries) => {
                self.list.lock().unwrap().insert(path.to_string(), entries);
            }
            Err(err) => {
                self.list_errors
                    .lock()
                    .unwrap()
                    .insert(path.to_string(), err.to_string());
            }
        }
    }

    pub(crate) fn set_home(&self, home: Option<String>) {
        *self.home.lock().unwrap() = home;
    }

    pub(crate) fn set_has_subdirs(&self, has: bool) {
        *self.has_subdirs.lock().unwrap() = has;
    }

    pub(crate) fn set_size(&self, result: Result<u64>) {
        match result {
            Ok(size) => {
                *self.size.lock().unwrap() = size;
                *self.size_error.lock().unwrap() = None;
            }
            Err(err) => {
                *self.size_error.lock().unwrap() = Some(err.to_string());
            }
        }
    }
}

#[cfg(test)]
impl SshBackend for MockSshBackend {
    fn list_remote_dir(
        &self,
        _open_connections: Option<&[OpenConnection]>,
        _conn: &ConnectionConfig,
        cwd: &str,
        _only_dirs: bool,
        _show_hidden: bool,
    ) -> Result<Vec<RemoteEntry>> {
        if let Some(err) = self.list_errors.lock().unwrap().get(cwd) {
            return Err(anyhow::anyhow!(err.to_string()));
        }
        Ok(self
            .list
            .lock()
            .unwrap()
            .get(cwd)
            .cloned()
            .unwrap_or_default())
    }

    fn remote_home_dir(
        &self,
        _open_connections: Option<&[OpenConnection]>,
        _conn: &ConnectionConfig,
    ) -> Result<Option<String>> {
        Ok(self.home.lock().unwrap().clone())
    }

    fn remote_has_subdirectories(
        &self,
        _open_connections: Option<&[OpenConnection]>,
        _conn: &ConnectionConfig,
        _path: &str,
    ) -> Result<bool> {
        Ok(*self.has_subdirs.lock().unwrap())
    }

    fn remote_size(&self, _conn: &ConnectionConfig, _path: &str, _is_dir: bool) -> Result<u64> {
        if let Some(err) = self.size_error.lock().unwrap().as_ref() {
            return Err(anyhow::anyhow!(err.clone()));
        }
        Ok(*self.size.lock().unwrap())
    }
}
