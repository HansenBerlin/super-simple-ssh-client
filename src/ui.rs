use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::model::{AuthConfig, AuthKind, Field, MasterField, Mode};

const HELP_TEXT: &str =
    "(t)erminal | (u)pload | (d)ownload | (o)ptions | (h)eader toggle | (q)uit";
const CONNECTION_COMMANDS: &str =
    "(n)ew | (e)dit | (c)onnect/(c)ancel | (d)ownload | (x)delete";
const LABEL_WIDTH: usize = 9;
const TRANSFER_PICKER_WIDTH: u16 = 60;
const TRANSFER_PICKER_HEIGHT: u16 = 90;

pub(crate) fn draw_ui(frame: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1)].as_ref())
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(layout[0]);

    let left = if app.show_header {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
            .split(body[0])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)].as_ref())
            .split(body[0])
    };

    if app.show_header {
        draw_app_header(frame, left[0]);
        draw_saved_list(frame, app, left[1]);
    } else {
        draw_saved_list(frame, app, left[0]);
    }
    draw_open_tabs(frame, app, body[1]);

    if app.mode == Mode::NewConnection {
        draw_new_connection_modal(frame, app);
        if app.try_result.is_some() {
            draw_try_result_modal(frame, app);
        }
    }
    if app.file_picker.is_some() {
        draw_file_picker_modal(frame, app);
    }
    if app.key_picker.is_some() {
        draw_key_picker_modal(frame, app);
    }
    if app.remote_picker.is_some() {
        draw_remote_picker_modal(frame, app);
    }
    if app.transfer.as_ref().is_some_and(|t| t.step == crate::model::TransferStep::Confirm) {
        draw_transfer_confirm_modal(frame, app);
    }
    if app.mode == Mode::ChangeMasterPassword {
        draw_master_password_modal(frame, app);
    }
    if app.mode == Mode::ConfirmDelete {
        draw_confirm_delete_modal(frame, app);
    }
    if app.notice.is_some() {
        draw_notice_modal(frame, app);
    }
}

fn draw_saved_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let header_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let connected: HashSet<String> = app
        .open_connections
        .iter()
        .map(|conn| crate::model::connection_key(&conn.config))
        .collect();
    let list_height = area.height.saturating_sub(2) as usize;
    let (start, end) = if app.connections.is_empty() || list_height == 0 {
        (0, 0)
    } else if app.selected_saved + 1 > list_height {
        let start = app.selected_saved + 1 - list_height;
        (start, (start + list_height).min(app.connections.len()))
    } else {
        (0, app.connections.len().min(list_height))
    };

    let items: Vec<ListItem> = if app.connections.is_empty() {
        vec![ListItem::new("No saved connections")]
    } else {
        app.connections[start..end]
            .iter()
            .map(|conn| {
                let key = crate::model::connection_key(conn);
                let status_style = if connected.contains(&key) {
                    Style::default().fg(Color::Green)
                } else if app.last_error.contains_key(&key) {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                let prefix = if connected.contains(&key) {
                    "  "
                } else if app.last_error.contains_key(&key) {
                    "! "
                } else {
                    "  "
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{prefix}{}", conn.label()),
                    status_style,
                )))
            })
            .collect()
    };

    let block = Block::default()
        .title(Line::from(Span::styled(
            "Available connections",
            header_style,
        )))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">");
    let list = list.highlight_symbol(Span::styled(
        ">",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ));
    let list_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };

    let mut state = ratatui::widgets::ListState::default();
    if app.connections.is_empty() {
        state.select(None);
    } else {
        let rel = app.selected_saved.saturating_sub(start);
        state.select(Some(rel));
    }
    frame.render_stateful_widget(list, list_area, &mut state);

    let commands = Paragraph::new(CONNECTION_COMMANDS)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);
    let commands_area = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    frame.render_widget(commands, commands_area);
}

fn draw_app_header(frame: &mut Frame<'_>, area: Rect) {
    let title = Paragraph::new("SUPER SIMPLE SSH 0.1.0")
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    frame.render_widget(title, area);
}

fn draw_open_tabs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let header_style = Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD);
    let tabs_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 3,
    };
    let (body_area, help_area) = if app.show_help {
        (
            Rect {
                x: area.x,
                y: area.y + 3,
                width: area.width,
                height: area.height.saturating_sub(3),
            },
            Some(tabs_area),
        )
    } else {
        (area, None)
    };
    if let Some(help_area) = help_area {
        let help = Paragraph::new(HELP_TEXT)
            .block(Block::default().title(Line::from(Span::styled(
                "Help",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ))).borders(Borders::ALL))
            .style(Style::default().fg(Color::Gray));
        frame.render_widget(help, help_area);
    }

    let connected: HashSet<String> = app
        .open_connections
        .iter()
        .map(|conn| crate::model::connection_key(&conn.config))
        .collect();
    let content = if let Some(conn) = app.connections.get(app.selected_saved) {
        let key = crate::model::connection_key(conn);
        let status = if connected.contains(&key) {
            "Connected"
        } else if app.last_error.contains_key(&key) {
            "Failed"
        } else if conn.history.is_empty() {
            "Never connected"
        } else {
            "Not connected"
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Name: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(conn.label()),
            ]),
            Line::from(vec![
                Span::styled("User: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&conn.user),
            ]),
            Line::from(vec![
                Span::styled("Host: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&conn.host),
            ]),
            Line::from(vec![
                Span::styled("Auth: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(match &conn.auth {
                    AuthConfig::Password { .. } => "Password",
                    AuthConfig::PrivateKey { password: None, .. } => "Private key",
                    AuthConfig::PrivateKey {
                        password: Some(_), ..
                    } => "Private key + password",
                }),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(status),
            ]),
        ];

        if let Some(err) = app.last_error.get(&key) {
            lines.push(Line::from(vec![
                Span::styled("Error: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(err, Style::default().fg(Color::Red)),
            ]));
        }

        lines.push(Line::from(""));
        let history_len = conn.history.len();
        let start_end = app.history_range(
            history_len,
            app.last_error.contains_key(&key),
        );
        let start = start_end.0;
        let end = start_end.1;
        let page_size = (end.saturating_sub(start)).max(1);
        let max_page = history_len.saturating_sub(1) / page_size;
        let page = app.history_page.min(max_page) + 1;
        let total_pages = max_page + 1;
        lines.push(Line::from(Span::styled(
            format!("Past connections ({page}/{total_pages}):"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if conn.history.is_empty() {
            lines.push(Line::from("  (none)"));
        } else {
            for entry in conn.history.iter().rev().skip(start).take(end - start) {
                lines.push(Line::from(format!(
                    "  {}",
                    crate::model::format_history_entry(entry)
                )));
            }
        }

        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Line::from(Span::styled(
                        "Connection details",
                        header_style,
                    ))),
            )
            .wrap(Wrap { trim: true })
    } else {
        Paragraph::new("No saved connection selected")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Line::from(Span::styled(
                        "Connection details",
                        header_style,
                    ))),
            )
            .alignment(Alignment::Center)
    };

    frame.render_widget(content, body_area);
}

fn draw_new_connection_modal(frame: &mut Frame<'_>, app: &App) {
    let mut footer_lines = Vec::new();
    footer_lines.push(Line::from(vec![
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" or "),
        Span::styled("Up/Down", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to move, "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to select, "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to cancel"),
    ]));
    if app.new_connection.auth_kind == AuthKind::PrivateKeyWithPassword {
        footer_lines.push(Line::from(Span::styled(
            "F2 to browse for key file | F3 to pick from recent keys",
            Style::default().fg(Color::Gray),
        )));
    }

    let area_width = (frame.area().width.saturating_mul(70) / 100)
        .min(frame.area().width.saturating_sub(2))
        .max(30);
    let pad = 1u16;
    let content_width = area_width.saturating_sub(2 + pad * 2);
    let value_width = content_width
        .saturating_sub((2 + LABEL_WIDTH as u16 + 2) as u16) as usize;
    let max_height = frame.area().height.saturating_mul(70) / 100;

    let title = if app.edit_index.is_some() {
        "Edit connection"
    } else {
        "New connection"
    };
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);

    let mut lines = Vec::new();
    let name_row;
    let user_row;
    let host_row;
    let auth_row;
    let mut key_row = None;
    let mut pass_row = None;
    let action_test_row;
    let action_save_row;
    let mut row_idx = 0usize;

    name_row = Some(row_idx);
    lines.push(field_line(
        "Name",
        &app.new_connection.name,
        app.new_connection.active_field == Field::Name,
        false,
        LABEL_WIDTH,
        value_width,
    ));
    row_idx += 1;

    user_row = Some(row_idx);
    lines.push(field_line(
        "User",
        &app.new_connection.user,
        app.new_connection.active_field == Field::User,
        false,
        LABEL_WIDTH,
        value_width,
    ));
    row_idx += 1;

    host_row = Some(row_idx);
    lines.push(field_line(
        "Host",
        &app.new_connection.host,
        app.new_connection.active_field == Field::Host,
        false,
        LABEL_WIDTH,
        value_width,
    ));
    row_idx += 1;

    auth_row = Some(row_idx);
    lines.push(field_line(
        "Auth",
        auth_kind_label(app.new_connection.auth_kind),
        app.new_connection.active_field == Field::AuthType,
        false,
        LABEL_WIDTH,
        value_width,
    ));
    row_idx += 1;
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PrivateKey | AuthKind::PrivateKeyWithPassword
    ) {
        key_row = Some(row_idx);
        lines.push(field_line(
            "Key path",
            &app.new_connection.key_path,
            app.new_connection.active_field == Field::KeyPath,
            false,
            LABEL_WIDTH,
            value_width,
        ));
        row_idx += 1;
    }
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PasswordOnly | AuthKind::PrivateKeyWithPassword
    ) {
        pass_row = Some(row_idx);
        lines.push(field_line(
            "Password",
            &app.new_connection.password,
            app.new_connection.active_field == Field::Password,
            true,
            LABEL_WIDTH,
            value_width,
        ));
        row_idx += 1;
    }

    lines.push(Line::from(""));
    row_idx += 1;
    action_test_row = Some(row_idx);
    lines.push(action_line(
        "Test connection",
        app.new_connection.active_field == Field::ActionTest,
    ));
    row_idx += 1;
    action_save_row = Some(row_idx);
    lines.push(action_line(
        "Save connection",
        app.new_connection.active_field == Field::ActionSave,
    ));

    if let Some(message) = &app.new_connection_feedback {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(Color::Red),
        )));
    }

    let content_lines = lines.len();
    let desired_height = modal_height(content_lines, footer_lines.len());
    let area_height = desired_height
        .max(10)
        .min(max_height.max(10))
        .min(frame.area().height.saturating_sub(2));
    let area = centered_rect_abs(area_width, area_height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let inner = padded_rect(area, pad);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(footer_lines.len() as u16),
        ]
        .as_ref())
        .split(inner);

    let active_row = match app.new_connection.active_field {
        Field::Name => name_row,
        Field::User => user_row,
        Field::Host => host_row,
        Field::AuthType => auth_row,
        Field::KeyPath => key_row,
        Field::Password => pass_row,
        Field::ActionTest => action_test_row,
        Field::ActionSave => action_save_row,
    };
    let max_visible = layout[0].height as usize;
    let scroll = if lines.len() > max_visible {
        let active_row = active_row.unwrap_or(0);
        let mut offset = if active_row + 1 > max_visible {
            active_row + 1 - max_visible
        } else {
            0
        };
        let max_offset = lines.len().saturating_sub(max_visible);
        if offset > max_offset {
            offset = max_offset;
        }
        offset
    } else {
        0
    };
    let visible_lines = if lines.len() > max_visible {
        lines[scroll..scroll + max_visible].to_vec()
    } else {
        lines
    };
    let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, layout[0]);
    render_input_cursor(
        frame,
        app,
        layout[0],
        scroll,
        name_row,
        user_row,
        host_row,
        auth_row,
        key_row,
        pass_row,
    );

    let footer = Paragraph::new(footer_lines).style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn draw_file_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.file_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(TRANSFER_PICKER_WIDTH, TRANSFER_PICKER_HEIGHT, frame.area());
    frame.render_widget(Clear, area);
    let title = if let Some(transfer) = &app.transfer {
        match (transfer.direction, transfer.step) {
            (crate::model::TransferDirection::Upload, crate::model::TransferStep::PickSource) => {
                "Pick source file or folder"
            }
            (crate::model::TransferDirection::Download, crate::model::TransferStep::PickTarget) => {
                "Pick target folder"
            }
            _ => "Pick key file",
        }
    } else {
        "Pick key file"
    };
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)].as_ref())
        .split(inner);

    let header = Paragraph::new(format!("Dir: {}", picker.cwd.display()))
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(header, layout[0]);

    let items: Vec<ListItem> = if picker.entries.is_empty() {
        vec![ListItem::new("Empty")]
    } else {
        picker
            .entries
            .iter()
            .map(|entry| {
                let suffix = if entry.is_dir { "/" } else { "" };
                ListItem::new(format!("{}{}", entry.name, suffix))
            })
            .collect()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled(">> ", Style::default().fg(Color::White)));
    frame.render_stateful_widget(
        list,
        layout[1],
        &mut list_state(picker.selected, picker.entries.len()),
    );

    let footer_text = if app.transfer.as_ref().is_some_and(|t| t.step == crate::model::TransferStep::PickSource) {
        "Enter to open, S to select folder, Backspace to up, Esc to cancel"
    } else if app.transfer.as_ref().is_some_and(|t| t.step == crate::model::TransferStep::PickTarget) {
        "Enter to open, S to select folder, Backspace to up, Esc to cancel"
    } else {
        "Enter to open/select, Backspace to up, Esc to cancel"
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[2]);
}

fn draw_key_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.key_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(TRANSFER_PICKER_WIDTH, TRANSFER_PICKER_HEIGHT, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Pick recent key",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let items: Vec<ListItem> = picker
        .keys
        .iter()
        .map(|entry| {
            let suffix = if entry.password.is_some() { " (pw)" } else { "" };
            ListItem::new(format!("{}{}", entry.path, suffix))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled(">> ", Style::default().fg(Color::White)));
    frame.render_stateful_widget(
        list,
        inner,
        &mut list_state(picker.selected, picker.keys.len()),
    );
}

fn draw_remote_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.remote_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);
    let title = if let Some(transfer) = &app.transfer {
        match (transfer.direction, transfer.step) {
            (crate::model::TransferDirection::Download, crate::model::TransferStep::PickSource) => {
                "Pick remote source"
            }
            _ => "Pick remote target",
        }
    } else {
        "Pick remote target"
    };
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)].as_ref())
        .split(inner);

    let header = Paragraph::new(format!("Dir: {}", picker.cwd))
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(header, layout[0]);

    let items: Vec<ListItem> = picker
        .entries
        .iter()
        .map(|entry| {
            let suffix = if entry.is_dir { "/" } else { "" };
            ListItem::new(format!("{}{}", entry.name, suffix))
        })
        .collect();

    if picker.loading {
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center);
        frame.render_widget(loading, layout[1]);
    } else if let Some(err) = &picker.error {
        let error = Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });
        frame.render_widget(error, layout[1]);
    } else {
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol(Span::styled(">> ", Style::default().fg(Color::White)));
        frame.render_stateful_widget(
            list,
            layout[1],
            &mut list_state(picker.selected, picker.entries.len()),
        );
    }

    let footer = Paragraph::new("Enter to open, S to select folder, Backspace to up, Esc to cancel")
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[2]);
}

fn draw_transfer_confirm_modal(frame: &mut Frame<'_>, app: &App) {
    let transfer = match &app.transfer {
        Some(transfer) => transfer,
        None => return,
    };
    let height = modal_height(3, 1);
    let area = centered_rect_by_height(70, height, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Confirm transfer",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let (source, target) = match transfer.direction {
        crate::model::TransferDirection::Upload => {
            let source = transfer
                .source_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "-".to_string());
            let target_dir = transfer
                .target_dir
                .clone()
                .unwrap_or_else(|| "-".to_string());
            let target_name = transfer
                .source_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| String::from("item"));
            let target = if target_dir == "-" {
                target_dir.clone()
            } else if target_dir == "/" {
                format!("/{target_name}")
            } else {
                format!("{target_dir}/{target_name}")
            };
            (source, target)
        }
        crate::model::TransferDirection::Download => {
            let source = transfer
                .source_remote
                .clone()
                .unwrap_or_else(|| "-".to_string());
            let target_dir = transfer
                .target_local_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "-".to_string());
            let target_name = transfer
                .source_remote
                .as_ref()
                .and_then(|p| std::path::Path::new(p).file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| String::from("item"));
            let target = if target_dir == "-" {
                target_dir.clone()
            } else {
                format!("{target_dir}/{target_name}")
            };
            (source, target)
        }
    };

    let lines = vec![
        Line::from(format!("Source: {source}")),
        Line::from(format!("Target: {target}")),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to transfer, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]),
    ];
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn draw_master_password_modal(frame: &mut Frame<'_>, app: &App) {
    let height = modal_height(6, 1);
    let area = centered_rect_by_height(60, height, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Change master password",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
        .split(inner);

    let value_width = layout[0]
        .width
        .saturating_sub((2 + LABEL_WIDTH as u16 + 2) as u16) as usize;
    let mut lines = Vec::new();
    let current_row = Some(0usize);
    let new_row = Some(1usize);
    let confirm_row = Some(2usize);

    lines.push(field_line(
        "Current",
        &app.master_change.current,
        app.master_change.active_field == MasterField::Current,
        true,
        LABEL_WIDTH,
        value_width,
    ));
    lines.push(field_line(
        "New",
        &app.master_change.new_password,
        app.master_change.active_field == MasterField::New,
        true,
        LABEL_WIDTH,
        value_width,
    ));
    lines.push(field_line(
        "Confirm",
        &app.master_change.confirm,
        app.master_change.active_field == MasterField::Confirm,
        true,
        LABEL_WIDTH,
        value_width,
    ));
    lines.push(Line::from(""));
    lines.push(action_line(
        "Save master password",
        app.master_change.active_field == MasterField::ActionSave,
    ));

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, layout[0]);
    render_master_cursor(
        frame,
        app,
        layout[0],
        current_row,
        new_row,
        confirm_row,
    );

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" or "),
        Span::styled("Up/Down", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to move, "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to select, "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to cancel"),
    ]))
    .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn draw_confirm_delete_modal(frame: &mut Frame<'_>, app: &App) {
    let height = modal_height(3, 0);
    let area = centered_rect_by_height(50, height, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Delete connection?",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let label = app
        .delete_index
        .and_then(|index| app.connections.get(index))
        .map(|conn| conn.label())
        .unwrap_or_else(|| "Unknown".to_string());

    let lines = vec![
        Line::from(format!("Delete {label}?")),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" or "),
            Span::styled("Y", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to confirm, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" or "),
            Span::styled("N", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn draw_try_result_modal(frame: &mut Frame<'_>, app: &App) {
    let result = match &app.try_result {
        Some(result) => result,
        None => return,
    };
    let height = modal_height(2, 1);
    let area = centered_rect_by_height(50, height, frame.area());
    frame.render_widget(Clear, area);
    let title = if result.success { "Try success" } else { "Try failed" };
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(2)].as_ref())
        .split(inner);

    let message = Paragraph::new(result.message.as_str()).wrap(Wrap { trim: true });
    frame.render_widget(message, layout[0]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("Press "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to close."),
    ]))
    .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn draw_notice_modal(frame: &mut Frame<'_>, app: &App) {
    let notice = match &app.notice {
        Some(notice) => notice,
        None => return,
    };
    let message_lines = notice.message.lines().count().max(1);
    let footer_lines = if app.notice_action_label().is_some() { 1 } else { 1 };
    let height = modal_height(message_lines + footer_lines, 0);
    let area = centered_rect_by_height(50, height, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            notice.title.as_str(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = padded_rect(area, 1);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(2)].as_ref())
        .split(inner);

    let message = Paragraph::new(notice.message.as_str()).wrap(Wrap { trim: true });
    frame.render_widget(message, layout[0]);

    let footer = if let Some(label) = app.notice_action_label() {
        Paragraph::new(Line::from(vec![
            Span::raw("Press "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to "),
            Span::raw(label),
            Span::raw(", "),
            Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to connect only, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to close."),
        ]))
        .style(Style::default().fg(Color::Gray))
    } else {
        Paragraph::new(Line::from(vec![
            Span::raw("Press "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to close."),
        ]))
        .style(Style::default().fg(Color::Gray))
    };
    frame.render_widget(footer, layout[1]);
}

fn field_line(
    label: &str,
    value: &str,
    active: bool,
    mask: bool,
    label_width: usize,
    max_value_width: usize,
) -> Line<'static> {
    let display = if mask && !value.is_empty() {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let display = truncate_text(&display, max_value_width);
    let indicator = if active { "> " } else { "  " };
    let indicator_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let spans = vec![
        Span::styled(indicator, indicator_style),
        Span::styled(
            format!("{label:<label_width$}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(display),
    ];
    Line::from(spans)
}

fn truncate_text(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let count = value.chars().count();
    if count <= max_width {
        return value.to_string();
    }
    if max_width <= 3 {
        return value.chars().take(max_width).collect();
    }
    let mut trimmed: String = value.chars().take(max_width - 3).collect();
    trimmed.push_str("...");
    trimmed
}

fn action_line(label: &str, active: bool) -> Line<'static> {
    let indicator = if active { "> " } else { "  " };
    let indicator_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let spans = vec![
        Span::styled(indicator, indicator_style),
        Span::styled(format!("{label}"), Style::default().add_modifier(Modifier::BOLD)),
    ];
    Line::from(spans)
}

fn render_input_cursor(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    scroll: usize,
    name_row: Option<usize>,
    user_row: Option<usize>,
    host_row: Option<usize>,
    _auth_row: Option<usize>,
    key_row: Option<usize>,
    pass_row: Option<usize>,
) {
    let (row, col) = match app.new_connection.active_field {
        Field::Name => (name_row, app.new_connection.name.chars().count()),
        Field::User => (user_row, app.new_connection.user.chars().count()),
        Field::Host => (host_row, app.new_connection.host.chars().count()),
        Field::AuthType => return,
        Field::KeyPath => (key_row, app.new_connection.key_path.chars().count()),
        Field::Password => (pass_row, app.new_connection.password.chars().count()),
        Field::ActionTest | Field::ActionSave => return,
    };
    let Some(row) = row else {
        return;
    };
    if row < scroll {
        return;
    }
    let visible_row = row.saturating_sub(scroll);
    if visible_row >= area.height as usize {
        return;
    }
    let indicator_len = 2u16;
    let label_len = LABEL_WIDTH as u16 + 2;
    let cursor_x = area.x + indicator_len + label_len + col as u16;
    let cursor_y = area.y + visible_row as u16;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn render_master_cursor(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    current_row: Option<usize>,
    new_row: Option<usize>,
    confirm_row: Option<usize>,
) {
    let (row, col) = match app.master_change.active_field {
        MasterField::Current => (current_row, app.master_change.current.chars().count()),
        MasterField::New => (new_row, app.master_change.new_password.chars().count()),
        MasterField::Confirm => (confirm_row, app.master_change.confirm.chars().count()),
        MasterField::ActionSave => return,
    };
    let Some(row) = row else {
        return;
    };
    if row >= area.height as usize {
        return;
    }
    let indicator_len = 2u16;
    let label_len = LABEL_WIDTH as u16 + 2;
    let cursor_x = area.x + indicator_len + label_len + col as u16;
    let cursor_y = area.y + row as u16;
    frame.set_cursor_position((cursor_x, cursor_y));
}



fn auth_kind_label(kind: AuthKind) -> &'static str {
    match kind {
        AuthKind::PasswordOnly => "Password only",
        AuthKind::PrivateKey => "Private key",
        AuthKind::PrivateKeyWithPassword => "Private key + password",
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

fn centered_rect_by_height(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    centered_rect_abs(width, height, area)
}

fn centered_rect_abs(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.max(10).min(area.width);
    let height = height.max(5).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

fn padded_rect(area: Rect, pad: u16) -> Rect {
    Rect {
        x: area.x + pad,
        y: area.y + pad,
        width: area.width.saturating_sub(pad * 2),
        height: area.height.saturating_sub(pad * 2),
    }
}

fn modal_height(content_lines: usize, footer_lines: usize) -> u16 {
    let total = content_lines + footer_lines;
    (total as u16).saturating_add(2 + 2)
}

fn list_state(selected: usize, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(selected.min(len.saturating_sub(1))));
    }
    state
}
