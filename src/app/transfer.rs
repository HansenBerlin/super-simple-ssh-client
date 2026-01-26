use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};

use anyhow::Result;

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
        let (source_path, source_is_dir, should_calc) = if let Some(transfer) = &mut self.transfer
        {
            transfer.target_dir = Some(path);
            transfer.step = TransferStep::Confirm;
            (
                transfer.source_path.clone(),
                transfer.source_is_dir,
                transfer.size_bytes.is_none(),
            )
        } else {
            return;
        };
        if should_calc {
            self.start_size_calc(move || compute_local_size(&source_path, source_is_dir));
        }
    }

    pub(crate) fn select_target_local_dir(&mut self, path: PathBuf) {
        let (source_remote, source_is_dir, should_calc) = if let Some(transfer) = &mut self.transfer
        {
            transfer.target_local_dir = Some(path);
            transfer.step = TransferStep::Confirm;
            (
                transfer.source_remote.clone(),
                transfer.source_is_dir,
                transfer.size_bytes.is_none(),
            )
        } else {
            return;
        };
        if should_calc {
            let Some(conn) = self.selected_connected_connection() else {
                self.set_status("Selected connection is not connected");
                return;
            };
            self.start_size_calc(move || {
                let session = connect_ssh(&conn)?;
                let Some(path) = source_remote else {
                    anyhow::bail!("missing remote source");
                };
                crate::ssh::remote_size(&session, &path, source_is_dir)
            });
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

    pub(crate) fn poll_size_calc(&mut self) {
        let Some(rx) = self.size_calc_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok((generation, result)) => {
                if generation == self.size_calc_generation {
                    match result {
                        Ok(size) => {
                            if let Some(transfer) = &mut self.transfer {
                                transfer.size_bytes = Some(size);
                            }
                        }
                        Err(err) => {
                            self.set_status(format!("Failed to compute size: {err}"));
                        }
                    }
                }
            }
            Err(TryRecvError::Empty) => {
                self.size_calc_rx = Some(rx);
            }
            Err(TryRecvError::Disconnected) => {}
        }
    }

    fn start_size_calc<F>(&mut self, job: F)
    where
        F: FnOnce() -> Result<u64> + Send + 'static,
    {
        self.size_calc_generation = self.size_calc_generation.wrapping_add(1);
        let generation = self.size_calc_generation;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = job();
            let _ = tx.send((generation, result));
        });
        self.size_calc_rx = Some(rx);
    }
}
