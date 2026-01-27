use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;

use crate::app::constants::{LOG_NO_LOGS_MESSAGE, STATUS_READY};
use crate::app::logging::prune_log_file;
use crate::model::{
    AppAction, ConnectionConfig, FilePickerState, KeyPickerState, MasterPasswordState, Mode,
    NewConnectionState, Notice, OpenConnection, RemoteEntry, RemotePickerState, TransferState,
    TransferUpdate, TryResult,
};
use crate::storage::{config_path, load_or_init_store, log_path};

mod constants;
mod connections;
mod handlers;
mod helpers;
mod logging;
mod pickers;
mod terminal;
mod transfer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderMode {
    Help,
    Logs,
    Off,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NoticeAction {
    ConnectTerminal,
    ConnectUpload,
    ConnectDownload,
}

pub(crate) struct App {
    pub(crate) config_path: PathBuf,
    pub(crate) log_path: PathBuf,
    pub(crate) last_log: String,
    pub(crate) log_lines: VecDeque<String>,
    pub(crate) last_local_dir: Option<PathBuf>,
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
    pub(super) notice_action: Option<NoticeAction>,
    pub(crate) header_mode: HeaderMode,
    pub(crate) history_page: usize,
    pub(crate) details_height: u16,
    pub(crate) transfer: Option<TransferState>,
    pub(crate) remote_picker: Option<RemotePickerState>,
    pub(crate) remote_fetch: Option<mpsc::Receiver<Result<Vec<RemoteEntry>>>>,
    pub(crate) transfer_progress: Option<mpsc::Receiver<TransferUpdate>>,
    pub(crate) transfer_cancel: Option<mpsc::Sender<()>>,
    pub(crate) transfer_hidden: bool,
    pub(crate) transfer_last_logged: u64,
    pub(crate) size_calc_rx: Option<mpsc::Receiver<(u64, Result<u64>)>>,
    pub(crate) size_calc_generation: u64,
    pub(crate) terminal_tabs: Vec<crate::app::terminal::TerminalTab>,
    pub(crate) active_terminal_tab: usize,
}

impl App {
    pub(crate) fn load_with_master() -> Result<Self> {
        let config_path = config_path()?;
        let (master, master_key, connections, last_local_dir) = load_or_init_store(&config_path)?;
        let log_path = log_path()?;
        prune_log_file(&log_path);
        let log_lines = VecDeque::new();
        let last_log = String::from(LOG_NO_LOGS_MESSAGE);
        let mut app = Self {
            config_path,
            log_path,
            last_log,
            log_lines,
            last_local_dir,
            master,
            master_key,
            connections,
            selected_saved: 0,
            selected_tab: 0,
            open_connections: vec![],
            mode: Mode::Normal,
            new_connection: NewConnectionState::default(),
            master_change: MasterPasswordState::default(),
            status: STATUS_READY.to_string(),
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
            size_calc_rx: None,
            size_calc_generation: 0,
            terminal_tabs: vec![],
            active_terminal_tab: 0,
        };
        app.sort_connections_by_recent(None);
        app.set_status(STATUS_READY);
        Ok(app)
    }

    pub(crate) fn set_details_height(&mut self, height: u16) {
        self.details_height = height;
    }
}
