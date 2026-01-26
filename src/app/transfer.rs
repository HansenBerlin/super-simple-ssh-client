use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::{Context, Result};

use crate::app::constants::{MB_BYTES, TRANSFER_LOG_THRESHOLD_BYTES};
use crate::app::helpers::compute_local_size;
use crate::app::App;
use crate::model::{
    ConnectionConfig, Notice, TransferDirection, TransferState, TransferStep, TransferUpdate,
};
use crate::ssh::connect_ssh;

impl App {
    pub(crate) fn start_upload(&mut self, conn: ConnectionConfig) {
        self.start_transfer(TransferDirection::Upload);
        if let Ok(start_dir) = crate::app::helpers::resolve_picker_start("") {
            if let Ok(entries) = crate::app::helpers::read_dir_entries_filtered(&start_dir, false) {
                self.file_picker = Some(crate::model::FilePickerState {
                    cwd: start_dir,
                    entries,
                    selected: 0,
                });
            }
        }
        self.set_status(format!("Select source for {}", conn.label()));
    }

    pub(crate) fn start_download(&mut self, conn: ConnectionConfig) {
        self.start_transfer(TransferDirection::Download);
        if let Err(err) = self.open_remote_picker() {
            self.set_status(format!("Failed to open remote picker: {err}"));
        }
        self.set_status(format!("Select remote source for {}", conn.label()));
    }

    fn start_transfer(&mut self, direction: TransferDirection) {
        self.transfer = Some(TransferState {
            direction,
            step: TransferStep::PickSource,
            source_path: None,
            source_remote: None,
            source_is_dir: false,
            target_dir: None,
            target_local_dir: None,
            size_bytes: None,
            progress_bytes: 0,
        });
    }

    pub(crate) fn select_source_path(&mut self, path: PathBuf, is_dir: bool) {
        if let Some(transfer) = &mut self.transfer {
            transfer.source_path = Some(path);
            transfer.source_is_dir = is_dir;
            transfer.step = TransferStep::PickTarget;
            transfer.size_bytes = None;
        }
    }

    pub(crate) fn select_source_remote(&mut self, path: String, is_dir: bool) {
        if let Some(transfer) = &mut self.transfer {
            transfer.source_remote = Some(path);
            transfer.source_is_dir = is_dir;
            transfer.step = TransferStep::PickTarget;
            transfer.size_bytes = None;
        }
    }

    pub(crate) fn select_target_dir(&mut self, path: String) {
        if let Some(transfer) = &mut self.transfer {
            transfer.target_dir = Some(path);
            transfer.step = TransferStep::Confirm;
            if transfer.size_bytes.is_none() {
                transfer.size_bytes =
                    compute_local_size(&transfer.source_path, transfer.source_is_dir).ok();
            }
        }
    }

    pub(crate) fn select_target_local_dir(&mut self, path: PathBuf) {
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

    pub(crate) fn start_transfer_job(&mut self) {
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
                        if transfer
                            .progress_bytes
                            .saturating_sub(self.transfer_last_logged)
                            >= TRANSFER_LOG_THRESHOLD_BYTES
                        {
                            let total = transfer.size_bytes.unwrap_or(0);
                            log_message = Some(if total == 0 {
                                format!("Transfer progress: {} B", transfer.progress_bytes)
                            } else {
                                let percent = (transfer.progress_bytes as f64 / total as f64 * 100.0)
                                    .round() as u64;
                                let current_mb =
                                    (transfer.progress_bytes as f64 / MB_BYTES).round() as u64;
                                let total_mb = (total as f64 / MB_BYTES).round() as u64;
                                format!(
                                    "Transfer progress: {percent}% ({current_mb} MB of {total_mb} MB)"
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
