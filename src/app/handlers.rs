use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as Base64;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::constants::{
    NOT_CONNECTED_MESSAGE, NOTICE_NO_SUBFOLDERS_MESSAGE, NOTICE_NO_SUBFOLDERS_TITLE,
    NOTICE_NOT_CONNECTED_MESSAGE, NOTICE_NOT_CONNECTED_TITLE, STATUS_CANCELLED,
};
use crate::app::{App, NoticeAction};
use crate::model::{
    AppAction, AuthKind, Field, MasterField, Mode, Notice, TransferDirection, TransferStep,
};
use crate::storage::{create_master_from_password, save_store};

impl App {
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

    fn cycle_header_mode(&mut self) {
        self.header_mode = match self.header_mode {
            crate::app::HeaderMode::Help => crate::app::HeaderMode::Logs,
            crate::app::HeaderMode::Logs => crate::app::HeaderMode::Off,
            crate::app::HeaderMode::Off => crate::app::HeaderMode::Help,
        };
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('n') => {
                self.mode = Mode::NewConnection;
                self.new_connection = crate::model::NewConnectionState::default();
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
            KeyCode::Char('v') => {
                self.cycle_header_mode();
            }
            KeyCode::Char('t') => {
                if self.selected_connected_connection().is_some() {
                    self.pending_action = Some(AppAction::OpenTerminal);
                } else {
                    self.notice = Some(Notice {
                        title: NOTICE_NOT_CONNECTED_TITLE.to_string(),
                        message: NOTICE_NOT_CONNECTED_MESSAGE.to_string(),
                    });
                    self.notice_action = Some(NoticeAction::ConnectTerminal);
                }
            }
            KeyCode::Char('u') => {
                if let Some(conn) = self.selected_connected_connection() {
                    self.start_upload(conn);
                } else {
                    self.notice = Some(Notice {
                        title: NOTICE_NOT_CONNECTED_TITLE.to_string(),
                        message: NOTICE_NOT_CONNECTED_MESSAGE.to_string(),
                    });
                    self.notice_action = Some(NoticeAction::ConnectUpload);
                }
            }
            KeyCode::Char('d') => {
                if let Some(conn) = self.selected_connected_connection() {
                    self.start_download(conn);
                } else {
                    self.notice = Some(Notice {
                        title: NOTICE_NOT_CONNECTED_TITLE.to_string(),
                        message: NOTICE_NOT_CONNECTED_MESSAGE.to_string(),
                    });
                    self.notice_action = Some(NoticeAction::ConnectDownload);
                }
            }
            KeyCode::Char('o') => {
                self.mode = Mode::ChangeMasterPassword;
                self.master_change = crate::model::MasterPasswordState::default();
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
                self.set_status(STATUS_CANCELLED);
            }
            KeyCode::Tab | KeyCode::Down => self.advance_field(true),
            KeyCode::BackTab | KeyCode::Up => self.advance_field(false),
            KeyCode::Left | KeyCode::Right => {
                if self.new_connection.active_field == Field::AuthType {
                    let next = match (self.new_connection.auth_kind, key.code) {
                        (AuthKind::PasswordOnly, KeyCode::Right) => AuthKind::PrivateKey,
                        (AuthKind::PrivateKey, KeyCode::Right) => AuthKind::PrivateKeyWithPassword,
                        (AuthKind::PrivateKeyWithPassword, KeyCode::Right) => {
                            AuthKind::PasswordOnly
                        }
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
            KeyCode::Enter => match self.new_connection.active_field {
                Field::ActionTest => self.run_test_connection(),
                Field::ActionSave => self.run_save_connection(),
                _ => {}
            },
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
                self.set_status(STATUS_CANCELLED);
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
                self.set_status(STATUS_CANCELLED);
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Some(index) = self.delete_index.take() {
                    if index < self.connections.len() {
                        let removed = self.connections.remove(index);
                        self.last_error
                            .remove(&crate::model::connection_key(&removed));
                        self.save_store()?;
                        if self.selected_saved >= self.connections.len() && self.selected_saved > 0 {
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

    fn handle_file_picker_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(picker) = &mut self.file_picker {
            let transfer_mode = self.transfer.as_ref().map(|t| (t.direction, t.step));
            let only_dirs = transfer_mode
                .map(|m| m.0 == TransferDirection::Download && m.1 == TransferStep::PickTarget)
                .unwrap_or(false);
            if matches!(key.code, KeyCode::Char('b')) {
                if let Some((direction, step)) = transfer_mode {
                    if matches!(step, TransferStep::PickTarget) {
                        if direction == TransferDirection::Download {
                            let start = self
                                .transfer
                                .as_ref()
                                .and_then(|t| t.source_remote.as_ref())
                                .map(|path| {
                                    std::path::Path::new(path)
                                        .parent()
                                        .map(|p| p.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "/".to_string())
                                });
                            if let Some(transfer) = &mut self.transfer {
                                transfer.step = TransferStep::PickSource;
                            }
                            self.file_picker = None;
                            if let Some(start) = start {
                                self.open_remote_picker_at(start, false)?;
                            } else {
                                self.open_remote_picker()?;
                            }
                        }
                    }
                    return Ok(false);
                }
            }
            match key.code {
                KeyCode::Esc => {
                    self.file_picker = None;
                    if self.transfer.is_some() {
                        self.transfer = None;
                        self.transfer_hidden = false;
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
                        picker.entries = crate::app::helpers::read_dir_entries_filtered(
                            &picker.cwd,
                            only_dirs,
                        )?;
                        picker.selected = 0;
                    }
                }
                KeyCode::Enter => {
                    if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                        if entry.is_dir {
                            if only_dirs {
                                let subdirectories = crate::app::helpers::read_dir_entries_filtered(
                                    &entry.path,
                                    true,
                                )?;
                                if subdirectories.is_empty() {
                                    self.notice = Some(Notice {
                                        title: NOTICE_NO_SUBFOLDERS_TITLE.to_string(),
                                        message: NOTICE_NO_SUBFOLDERS_MESSAGE.to_string(),
                                    });
                                    return Ok(false);
                                }
                            }
                            picker.cwd = entry.path;
                            picker.entries = crate::app::helpers::read_dir_entries_filtered(
                                &picker.cwd,
                                only_dirs,
                            )?;
                            picker.selected = 0;
                        } else if let Some((direction, step)) = transfer_mode {
                            if direction == TransferDirection::Upload
                                && step == TransferStep::PickSource
                            {
                                self.last_local_dir = Some(picker.cwd.clone());
                                self.save_store()?;
                                self.select_source_path(entry.path, false);
                                self.file_picker = None;
                                self.open_remote_picker()?;
                            }
                        } else {
                            self.new_connection.key_path = entry.path.to_string_lossy().into_owned();
                            self.file_picker = None;
                        }
                    }
                }
                KeyCode::Char('s') => {
                    if let Some(entry) = picker.entries.get(picker.selected).cloned() {
                        if entry.is_dir {
                            match transfer_mode {
                                Some((TransferDirection::Upload, TransferStep::PickSource)) => {
                                    self.last_local_dir = Some(entry.path.clone());
                                    self.save_store()?;
                                    self.select_source_path(entry.path, true);
                                    self.file_picker = None;
                                    self.open_remote_picker()?;
                                }
                                Some((TransferDirection::Download, TransferStep::PickTarget)) => {
                                    self.last_local_dir = Some(entry.path.clone());
                                    self.save_store()?;
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
        let transfer_mode = self.transfer.as_ref().map(|t| (t.direction, t.step));
        let only_dirs = picker.only_dirs;
        match key.code {
            KeyCode::Esc => {
                self.remote_picker = None;
                if self.transfer.is_some() {
                    self.transfer = None;
                    self.transfer_hidden = false;
                }
                return Ok(false);
            }
            KeyCode::Char('b') => {
                if let Some((direction, step)) = transfer_mode {
                    if direction == TransferDirection::Upload && step == TransferStep::PickTarget {
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
                    let new_cwd = crate::app::helpers::parent_remote_dir(&picker.cwd);
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
                                self.set_status(NOT_CONNECTED_MESSAGE);
                                self.remote_picker = Some(picker);
                                return Ok(false);
                            };
                            let open_conn = match self.open_connections.iter().find(|candidate| {
                                crate::model::same_identity(&candidate.config, &conn)
                            }) {
                                Some(conn) => conn,
                                None => {
                                    self.set_status(NOT_CONNECTED_MESSAGE);
                                    self.remote_picker = Some(picker);
                                    return Ok(false);
                                }
                            };
                            if !crate::ssh::remote_has_subdirectories(
                                &open_conn.session,
                                &entry.path,
                            )? {
                                self.notice = Some(Notice {
                                    title: NOTICE_NO_SUBFOLDERS_TITLE.to_string(),
                                    message: NOTICE_NO_SUBFOLDERS_MESSAGE.to_string(),
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
                        self.update_last_remote_dir(picker.cwd.clone())?;
                        self.select_source_remote(entry.path, false);
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
                                self.update_last_remote_dir(entry.path.clone())?;
                                self.select_target_dir(entry.path);
                                return Ok(false);
                            }
                            Some((TransferDirection::Download, TransferStep::PickSource)) => {
                                self.update_last_remote_dir(entry.path.clone())?;
                                self.select_source_remote(entry.path, true);
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
                self.transfer_hidden = false;
                self.set_status(STATUS_CANCELLED);
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
                        if let Some(transfer) = &self.transfer {
                            let start = transfer.target_dir.clone().unwrap_or_else(|| "/".to_string());
                            self.open_remote_picker_at(start, true)?;
                        }
                    }
                    Some(TransferDirection::Download) => {
                        if let Some(transfer) = &self.transfer {
                            self.open_local_target_picker_at(transfer.target_local_dir.clone())?;
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
        Ok(false)
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
                    if let Some(key) = picker.keys.get(picker.selected) {
                        self.new_connection.key_path = key.path.clone();
                        if let Some(password) = &key.password {
                            self.new_connection.password = password.clone();
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

    fn advance_field(&mut self, forward: bool) {
        let fields = self.active_fields();
        if let Some(pos) = fields
            .iter()
            .position(|field| *field == self.new_connection.active_field)
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

        let salt = Base64
            .decode(&self.master.salt_b64)
            .context("decode salt")?;
        let current_key = crate::storage::derive_key(&self.master_change.current, &salt);
        let check = crate::storage::decrypt_string(&self.master.check, &current_key)
            .context("verify current password")?;
        if check != "ssh-client-check" {
            anyhow::bail!("Current master password incorrect");
        }

        let (new_master, new_key) = create_master_from_password(&self.master_change.new_password)?;
        let stored = crate::model::StoreFile {
            master: new_master.clone(),
            connections: self
                .connections
                .iter()
                .map(|conn| crate::storage::encrypt_connection(conn, &new_key))
                .collect::<Result<Vec<_>>>()?,
            last_local_dir: self
                .last_local_dir
                .as_ref()
                .map(|value| value.to_string_lossy().into_owned()),
        };
        save_store(&self.config_path, &stored)?;
        self.master = new_master;
        self.master_key = new_key;
        self.master_change = crate::model::MasterPasswordState::default();
        Ok(())
    }
}

enum EditAction {
    Insert(char),
    Backspace,
}
