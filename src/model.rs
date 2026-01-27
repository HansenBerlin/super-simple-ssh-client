use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::Local;
use serde::{Deserialize, Serialize};
use ssh2::Session;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConnectionConfig {
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) user: String,
    pub(crate) host: String,
    pub(crate) auth: AuthConfig,
    #[serde(default)]
    pub(crate) history: Vec<HistoryEntry>,
    #[serde(default)]
    pub(crate) last_remote_dir: Option<String>,
}

impl ConnectionConfig {
    pub(crate) fn label(&self) -> String {
        if self.name.trim().is_empty() {
            self.host.clone()
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum AuthConfig {
    Password {
        password: String,
    },
    PrivateKey {
        path: String,
        password: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum HistoryState {
    Success,
    Failure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HistoryEntry {
    pub(crate) ts: u64,
    pub(crate) state: HistoryState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoreFile {
    pub(crate) master: MasterConfig,
    pub(crate) connections: Vec<StoredConnection>,
    #[serde(default)]
    pub(crate) last_local_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MasterConfig {
    pub(crate) salt_b64: String,
    pub(crate) check: EncryptedBlob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EncryptedBlob {
    pub(crate) nonce: String,
    pub(crate) ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredConnection {
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) user: String,
    pub(crate) host: String,
    pub(crate) auth: StoredAuthConfig,
    #[serde(default, deserialize_with = "deserialize_history")]
    pub(crate) history: Vec<HistoryEntry>,
    #[serde(default)]
    pub(crate) last_remote_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StoredAuthConfig {
    Password {
        password: EncryptedBlob,
    },
    PrivateKey {
        path: String,
        password: Option<EncryptedBlob>,
    },
}

pub(crate) fn same_identity(left: &ConnectionConfig, right: &ConnectionConfig) -> bool {
    if left.user != right.user || left.host != right.host {
        return false;
    }
    match (&left.auth, &right.auth) {
        (AuthConfig::Password { .. }, AuthConfig::Password { .. }) => true,
        (
            AuthConfig::PrivateKey {
                path: left_path, ..
            },
            AuthConfig::PrivateKey {
                path: right_path, ..
            },
        ) => left_path == right_path,
        _ => false,
    }
}

pub(crate) fn connection_key(conn: &ConnectionConfig) -> String {
    let auth_key = match &conn.auth {
        AuthConfig::Password { .. } => "pw".to_string(),
        AuthConfig::PrivateKey { path, .. } => format!("pk:{}", path),
    };
    format!("{}@{}|{}", conn.user, conn.host, auth_key)
}

#[derive(Clone)]
pub(crate) struct OpenConnection {
    pub(crate) config: ConnectionConfig,
    pub(crate) session: Session,
    #[allow(dead_code)]
    pub(crate) connected_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    NewConnection,
    ChangeMasterPassword,
    ConfirmDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Name,
    User,
    Host,
    AuthType,
    KeyPath,
    Password,
    ActionTest,
    ActionSave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MasterField {
    Current,
    New,
    Confirm,
    ActionSave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthKind {
    PasswordOnly,
    PrivateKey,
    PrivateKeyWithPassword,
}

#[derive(Debug, Clone)]
pub(crate) struct NewConnectionState {
    pub(crate) name: String,
    pub(crate) user: String,
    pub(crate) host: String,
    pub(crate) auth_kind: AuthKind,
    pub(crate) key_path: String,
    pub(crate) password: String,
    pub(crate) active_field: Field,
}

impl Default for NewConnectionState {
    fn default() -> Self {
        Self {
            name: String::new(),
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
pub(crate) struct MasterPasswordState {
    pub(crate) current: String,
    pub(crate) new_password: String,
    pub(crate) confirm: String,
    pub(crate) active_field: MasterField,
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
pub(crate) struct FileEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FilePickerState {
    pub(crate) cwd: PathBuf,
    pub(crate) entries: Vec<FileEntry>,
    pub(crate) selected: usize,
    pub(crate) show_hidden: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RemotePickerState {
    pub(crate) cwd: String,
    pub(crate) entries: Vec<RemoteEntry>,
    pub(crate) selected: usize,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) only_dirs: bool,
    pub(crate) show_hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferStep {
    PickSource,
    PickTarget,
    Confirm,
    Transferring,
}

#[derive(Debug, Clone)]
pub(crate) struct TransferState {
    pub(crate) direction: TransferDirection,
    pub(crate) step: TransferStep,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) source_remote: Option<String>,
    pub(crate) source_is_dir: bool,
    pub(crate) target_dir: Option<String>,
    pub(crate) target_local_dir: Option<PathBuf>,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) progress_bytes: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct KeyPickerState {
    pub(crate) keys: Vec<KeyCandidate>,
    pub(crate) selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferDirection {
    Upload,
    Download,
}

pub(crate) enum TransferUpdate {
    Bytes(u64),
    Done(Result<(), String>),
}

#[derive(Debug, Clone)]
pub(crate) struct KeyCandidate {
    pub(crate) path: String,
    pub(crate) password: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TryResult {
    pub(crate) success: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone)]
pub(crate) enum AppAction {
    OpenTerminal,
}

#[derive(Debug, Clone)]
pub(crate) struct Notice {
    pub(crate) title: String,
    pub(crate) message: String,
}

pub(crate) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn format_history_entry(entry: &HistoryEntry) -> String {
    let dt =
        chrono::DateTime::<Local>::from(SystemTime::UNIX_EPOCH + Duration::from_secs(entry.ts));
    let state = match entry.state {
        HistoryState::Success => "success",
        HistoryState::Failure => "failed",
    };
    format!("{} | {}", dt.format("%Y-%m-%d %H:%M:%S"), state)
}

pub(crate) fn deserialize_history<'de, D>(deserializer: D) -> Result<Vec<HistoryEntry>, D::Error>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_prefers_name_over_host() {
        let mut conn = ConnectionConfig {
            name: String::new(),
            user: "u".to_string(),
            host: "host".to_string(),
            auth: AuthConfig::Password {
                password: "pw".to_string(),
            },
            history: vec![],
            last_remote_dir: None,
        };
        assert_eq!(conn.label(), "host");
        conn.name = "friendly".to_string();
        assert_eq!(conn.label(), "friendly");
    }

    #[test]
    fn same_identity_matches_auth_type() {
        let base = ConnectionConfig {
            name: String::new(),
            user: "u".to_string(),
            host: "h".to_string(),
            auth: AuthConfig::Password {
                password: "pw".to_string(),
            },
            history: vec![],
            last_remote_dir: None,
        };
        let other = ConnectionConfig {
            auth: AuthConfig::PrivateKey {
                path: "/k".to_string(),
                password: None,
            },
            ..base.clone()
        };
        assert!(!same_identity(&base, &other));
        let other_pw = ConnectionConfig {
            auth: AuthConfig::Password {
                password: "pw2".to_string(),
            },
            ..base.clone()
        };
        assert!(same_identity(&base, &other_pw));
    }

    #[test]
    fn connection_key_includes_auth_hint() {
        let conn = ConnectionConfig {
            name: String::new(),
            user: "u".to_string(),
            host: "h".to_string(),
            auth: AuthConfig::Password {
                password: "pw".to_string(),
            },
            history: vec![],
            last_remote_dir: None,
        };
        assert!(connection_key(&conn).contains("u@h|pw"));
    }

    #[test]
    fn deserialize_history_accepts_timestamps() {
        let json = r#"
        {
          "name": "",
          "user": "u",
          "host": "h",
          "auth": { "Password": { "password": { "nonce": "a", "ciphertext": "b" } } },
          "history": [1,2]
        }
        "#;
        let stored: StoredConnection = serde_json::from_str(json).unwrap();
        assert_eq!(stored.history.len(), 2);
        assert!(matches!(stored.history[0].state, HistoryState::Success));
    }

    #[test]
    fn format_history_entry_includes_state() {
        let entry = HistoryEntry {
            ts: 0,
            state: HistoryState::Failure,
        };
        let formatted = format_history_entry(&entry);
        assert!(formatted.contains("failed"));
        assert!(formatted.contains(" | "));
    }
}
