use std::io::{Read, Write};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ssh2::Session;

use crate::app::constants::NOT_CONNECTED_MESSAGE;
use crate::app::App;
use crate::ssh::{connect_ssh, terminal_key_bytes};

pub(crate) struct TerminalTab {
    pub(crate) title: String,
    pub(crate) _session: Session,
    pub(crate) channel: ssh2::Channel,
    pub(crate) parser: vt100::Parser,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) pending_write: Vec<u8>,
}

impl App {
    pub(crate) fn open_terminal_tab(&mut self, cols: u16, rows: u16) -> Result<()> {
        let Some(conn) = self.selected_connected_connection() else {
            self.set_status(NOT_CONNECTED_MESSAGE);
            return Ok(());
        };
        let session = connect_ssh(&conn)?;
        session.set_blocking(true);
        let mut channel = session.channel_session()?;
        channel.request_pty(
            "xterm-256color",
            None,
            Some((u32::from(cols.max(1)), u32::from(rows.max(1)), 0, 0)),
        )?;
        channel.shell()?;
        session.set_blocking(false);
        let parser = vt100::Parser::new(rows.max(1), cols.max(1), 0);
        let tab = TerminalTab {
            title: conn.label(),
            _session: session,
            channel,
            parser,
            cols: cols.max(1),
            rows: rows.max(1),
            pending_write: Vec::new(),
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
                tab.pending_write.extend_from_slice(&bytes);
            }
        }
        Ok(true)
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
}
