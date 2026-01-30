use std::io::{Read, Write};

use anyhow::Result;
use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ssh2::Session;

use crate::app::constants::NOT_CONNECTED_MESSAGE;
use crate::app::App;
use crate::ui::constants::{HEADER_HEIGHT, TERMINAL_FOOTER_HEIGHT};
use crate::ssh::{connect_ssh, terminal_key_bytes};

const TERMINAL_SCROLLBACK_LINES: u16 = 2000;
const TERMINAL_SCROLL_STEP: u16 = 3;

pub(crate) struct SelectionRange {
    pub(crate) start_row: u16,
    pub(crate) start_col: u16,
    pub(crate) end_row: u16,
    pub(crate) end_col: u16,
}

pub(crate) struct TerminalTab {
    pub(crate) title: String,
    pub(crate) _session: Session,
    pub(crate) channel: ssh2::Channel,
    pub(crate) parser: vt100::Parser,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) pending_write: Vec<u8>,
    pub(crate) selection_start: Option<(u16, u16)>,
    pub(crate) selection_end: Option<(u16, u16)>,
    pub(crate) selecting: bool,
}

impl App {
    pub(crate) fn open_terminal_tab(&mut self, cols: u16, rows: u16) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            self.set_status(NOT_CONNECTED_MESSAGE);
            return Ok(());
        };
        let session = connect_ssh(&conn)?;
        session.set_blocking(true);
        let view_cols = cols.max(1);
        let view_rows = rows.max(1);
        let mut channel = session.channel_session()?;
        channel.request_pty(
            "xterm-256color",
            None,
            Some((u32::from(view_cols), u32::from(view_rows), 0, 0)),
        )?;
        channel.shell()?;
        session.set_blocking(false);
        let parser = vt100::Parser::new(view_rows, view_cols, TERMINAL_SCROLLBACK_LINES.into());
        let tab = TerminalTab {
            title: conn.label(),
            _session: session,
            channel,
            parser,
            cols: view_cols,
            rows: view_rows,
            pending_write: Vec::new(),
            selection_start: None,
            selection_end: None,
            selecting: false,
        };
        self.terminal_tabs.push(tab);
        self.active_terminal_tab = self.terminal_tabs.len();
        Ok(())
    }

    pub(crate) fn terminal_tabs_open(&self) -> bool {
        !self.terminal_tabs.is_empty()
    }

    pub(crate) fn handle_terminal_tabs_key(&mut self, key: KeyEvent) -> Result<bool> {
        if !self.terminal_tabs_open() {
            return Ok(false);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if self.active_terminal_tab > 0 {
            if ctrl {
                if let KeyCode::Char(c) = key.code {
                    let c = c.to_ascii_lowercase();
                    if c == 'c' {
                        if self.terminal_has_selection() || shift {
                            self.copy_terminal_selection();
                            return Ok(true);
                        }
                    }
                    if c == 'v' {
                        self.paste_terminal_clipboard();
                        return Ok(true);
                    }
                }
            }
        }
        match key.code {
            KeyCode::F(6) => {
                self.cycle_terminal_tab_back();
                return Ok(true);
            }
            KeyCode::F(7) => {
                self.cycle_terminal_tab();
                return Ok(true);
            }
            KeyCode::F(8) => {
                self.close_active_terminal_tab();
                return Ok(true);
            }
            KeyCode::PageUp => {
                self.adjust_terminal_scrollback(TERMINAL_SCROLL_STEP as i16);
                return Ok(true);
            }
            KeyCode::PageDown => {
                self.adjust_terminal_scrollback(-(TERMINAL_SCROLL_STEP as i16));
                return Ok(true);
            }
            _ => {}
        }
        if self.active_terminal_tab == 0 {
            return Ok(false);
        }
        if let Some(bytes) = terminal_key_bytes(key) {
            if let Some(tab) = self
                .terminal_tabs
                .get_mut(self.active_terminal_tab.saturating_sub(1))
            {
                tab.selecting = false;
                tab.selection_start = None;
                tab.selection_end = None;
                tab.pending_write.extend_from_slice(&bytes);
                tab.parser.screen_mut().set_scrollback(0);
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_terminal_mouse(
        &mut self,
        mouse: MouseEvent,
        cols: u16,
        rows: u16,
    ) -> bool {
        if !self.terminal_tabs_open() || self.active_terminal_tab == 0 {
            return false;
        }
        let header = HEADER_HEIGHT;
        let footer = TERMINAL_FOOTER_HEIGHT;
        if rows <= header + footer {
            return false;
        }
        let view_rows = rows.saturating_sub(header + footer);
        let view_cols = cols.max(1);
        let view_top = header;
        let view_bottom = rows.saturating_sub(footer);
        if mouse.row < view_top || mouse.row >= view_bottom {
            if let Some(tab) = self
                .terminal_tabs
                .get_mut(self.active_terminal_tab.saturating_sub(1))
            {
                tab.selecting = false;
                tab.selection_start = None;
                tab.selection_end = None;
            }
            return false;
        }
        let row = mouse
            .row
            .saturating_sub(view_top)
            .min(view_rows.saturating_sub(1));
        let col = mouse
            .column
            .saturating_sub(1)
            .min(view_cols.saturating_sub(1));
        let col_exclusive = col.saturating_add(1).min(view_cols);

        if let Some(tab) = self
            .terminal_tabs
            .get_mut(self.active_terminal_tab.saturating_sub(1))
        {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    tab.selecting = false;
                    tab.selection_start = None;
                    tab.selection_end = None;
                    Self::adjust_scrollback_for(tab, TERMINAL_SCROLL_STEP as i16);
                    return true;
                }
                MouseEventKind::ScrollDown => {
                    tab.selecting = false;
                    tab.selection_start = None;
                    tab.selection_end = None;
                    Self::adjust_scrollback_for(tab, -(TERMINAL_SCROLL_STEP as i16));
                    return true;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    tab.selecting = true;
                    tab.selection_start = Some((row, col));
                    tab.selection_end = Some((row, col_exclusive));
                    return true;
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if tab.selecting {
                        tab.selection_end = Some((row, col_exclusive));
                        return true;
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if tab.selecting {
                        tab.selecting = false;
                        tab.selection_end = Some((row, col_exclusive));
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    pub(crate) fn poll_terminal_output(&mut self) {
        let mut buffer = [0u8; 4096];
        let mut err_buffer = [0u8; 1024];
        let mut closed = Vec::new();
        for (index, tab) in self.terminal_tabs.iter_mut().enumerate() {
            if !tab.pending_write.is_empty() {
                match tab.channel.write(&tab.pending_write) {
                    Ok(0) => {}
                    Ok(count) => {
                        tab.pending_write.drain(0..count);
                    }
                    Err(err) => {
                        if err.kind() != std::io::ErrorKind::WouldBlock {
                            closed.push(index);
                        }
                    }
                }
            }
            loop {
                match tab.channel.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        tab.parser.process(&buffer[..count]);
                    }
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::WouldBlock {
                            break;
                        }
                        break;
                    }
                }
            }
            loop {
                match tab.channel.stderr().read(&mut err_buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        tab.parser.process(&err_buffer[..count]);
                    }
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::WouldBlock {
                            break;
                        }
                        break;
                    }
                }
            }
            if tab.channel.eof() {
                closed.push(index);
            }
        }
        for index in closed.into_iter().rev() {
            self.terminal_tabs.remove(index);
        }
        if self.terminal_tabs.is_empty() {
            self.active_terminal_tab = 0;
        } else if self.active_terminal_tab > self.terminal_tabs.len() {
            self.active_terminal_tab = self.terminal_tabs.len();
        }
    }

    pub(crate) fn update_terminal_sizes(&mut self, cols: u16, rows: u16) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        for tab in &mut self.terminal_tabs {
            if tab.cols == cols && tab.rows == rows {
                continue;
            }
            tab.cols = cols;
            tab.rows = rows;
            tab.channel
                .request_pty_size(u32::from(cols), u32::from(rows), None, None)
                .ok();
            tab.parser.screen_mut().set_size(rows, cols);
        }
    }

    fn adjust_terminal_scrollback(&mut self, delta: i16) {
        if let Some(tab) = self
            .terminal_tabs
            .get_mut(self.active_terminal_tab.saturating_sub(1))
        {
            Self::adjust_scrollback_for(tab, delta);
        }
    }

    fn adjust_scrollback_for(tab: &mut TerminalTab, delta: i16) {
        let screen = tab.parser.screen_mut();
        let current = screen.scrollback();
        let next = if delta >= 0 {
            current.saturating_add(delta as usize)
        } else {
            current.saturating_sub(delta.unsigned_abs() as usize)
        };
        screen.set_scrollback(next);
    }

    fn cycle_terminal_tab(&mut self) {
        if self.terminal_tabs.is_empty() {
            self.active_terminal_tab = 0;
            return;
        }
        let total = self.terminal_tabs.len() + 1;
        self.active_terminal_tab = (self.active_terminal_tab + 1) % total;
    }

    fn cycle_terminal_tab_back(&mut self) {
        if self.terminal_tabs.is_empty() {
            self.active_terminal_tab = 0;
            return;
        }
        let total = self.terminal_tabs.len() + 1;
        if self.active_terminal_tab == 0 {
            self.active_terminal_tab = total - 1;
        } else {
            self.active_terminal_tab -= 1;
        }
    }

    fn close_active_terminal_tab(&mut self) {
        if self.active_terminal_tab == 0 {
            return;
        }
        let index = self.active_terminal_tab - 1;
        if let Some(tab) = self.terminal_tabs.get_mut(index) {
            tab.channel.close().ok();
        }
        self.terminal_tabs.remove(index);
        if self.terminal_tabs.is_empty() {
            self.active_terminal_tab = 0;
        } else if self.active_terminal_tab > self.terminal_tabs.len() {
            self.active_terminal_tab = self.terminal_tabs.len();
        }
    }

    fn terminal_has_selection(&self) -> bool {
        self.terminal_tabs
            .get(self.active_terminal_tab.saturating_sub(1))
            .is_some_and(|tab| tab.selection_range().is_some())
    }

    fn copy_terminal_selection(&mut self) {
        let Some(tab) = self
            .terminal_tabs
            .get(self.active_terminal_tab.saturating_sub(1))
        else {
            return;
        };
        let Some(text) = tab.selection_text() else {
            return;
        };
        if let Err(err) = self.copy_to_clipboard(&text) {
            self.set_status(&format!("Copy failed: {err}"));
        }
    }

    fn paste_terminal_clipboard(&mut self) {
        let Ok(text) = self.read_clipboard() else {
            self.set_status("Paste failed: clipboard unavailable");
            return;
        };
        if text.is_empty() {
            return;
        }
        let Some(tab) = self
            .terminal_tabs
            .get_mut(self.active_terminal_tab.saturating_sub(1))
        else {
            return;
        };
        let mut bytes = text.replace("\r\n", "\n").replace('\n', "\r").into_bytes();
        tab.pending_write.append(&mut bytes);
        tab.parser.screen_mut().set_scrollback(0);
    }

    fn clipboard_mut(&mut self) -> Result<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new()?);
        }
        Ok(self.clipboard.as_mut().expect("clipboard init"))
    }

    fn copy_to_clipboard(&mut self, text: &str) -> Result<()> {
        let clipboard = self.clipboard_mut()?;
        clipboard.set_text(text.to_string())?;
        Ok(())
    }

    fn read_clipboard(&mut self) -> Result<String> {
        let clipboard = self.clipboard_mut()?;
        clipboard.get_text().map_err(Into::into)
    }
}

impl TerminalTab {
    pub(crate) fn selection_range(&self) -> Option<SelectionRange> {
        let (start_row, start_col) = self.selection_start?;
        let (end_row, end_col) = self.selection_end.unwrap_or((start_row, start_col));
        if (start_row, start_col) <= (end_row, end_col) {
            Some(SelectionRange {
                start_row,
                start_col,
                end_row,
                end_col,
            })
        } else {
            Some(SelectionRange {
                start_row: end_row,
                start_col: end_col,
                end_row: start_row,
                end_col: start_col,
            })
        }
    }

    fn selection_text(&self) -> Option<String> {
        let mut range = self.selection_range()?;
        if range.start_col > 0 {
            range.start_col = range.start_col.saturating_sub(1);
        }
        let cols = self.parser.screen().size().1;
        if range.end_col < cols {
            range.end_col = range.end_col.saturating_add(1).min(cols);
        }
        if range.start_row == range.end_row && range.end_col == range.start_col {
            range.end_col = range.end_col.saturating_add(1).min(cols);
        }
        let screen = self.parser.screen();
        let text = screen.contents_between(
            range.start_row,
            range.start_col,
            range.end_row,
            range.end_col,
        );
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
