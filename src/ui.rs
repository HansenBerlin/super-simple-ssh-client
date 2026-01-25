use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::model::{AuthConfig, AuthKind, Field, MasterField, Mode};

const HELP_TEXT: &str =
    "n = new | e = edit | c = connect | d = disconnect | t = terminal | m = master pw | x = delete | h = toggle header | q = quit";
const LABEL_WIDTH: usize = 9;

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
        if app.file_picker.is_some() {
            draw_file_picker_modal(frame, app);
        }
        if app.key_picker.is_some() {
            draw_key_picker_modal(frame, app);
        }
        if app.try_result.is_some() {
            draw_try_result_modal(frame, app);
        }
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

    let list = List::new(items)
        .block(Block::default().title(Line::from(Span::styled(
            "Available connections",
            header_style,
        ))).borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">");
    let list = list.highlight_symbol(Span::styled(
        ">",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));

    let mut state = ratatui::widgets::ListState::default();
    if app.connections.is_empty() {
        state.select(None);
    } else {
        let rel = app.selected_saved.saturating_sub(start);
        state.select(Some(rel));
    }
    frame.render_stateful_widget(list, area, &mut state);
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

        let history_len = conn.history.len();
        let start_end = app.history_range(
            history_len,
            app.last_error.contains_key(&key),
        );
        let start = start_end.0;
        let end = start_end.1;

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Past connections:",
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
    let area = centered_rect(70, 70, frame.area());
    frame.render_widget(Clear, area);
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
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)].as_ref())
        .split(inner);

    let mut lines = Vec::new();
    let user_row;
    let host_row;
    let auth_row;
    let mut key_row = None;
    let mut pass_row = None;
    let mut row_idx = 0usize;

    user_row = Some(row_idx);
    lines.push(field_line(
        "User",
        &app.new_connection.user,
        app.new_connection.active_field == Field::User,
        false,
        LABEL_WIDTH,
    ));
    row_idx += 1;

    host_row = Some(row_idx);
    lines.push(field_line(
        "Host",
        &app.new_connection.host,
        app.new_connection.active_field == Field::Host,
        false,
        LABEL_WIDTH,
    ));
    row_idx += 1;

    auth_row = Some(row_idx);
    lines.push(field_line(
        "Auth",
        auth_kind_label(app.new_connection.auth_kind),
        app.new_connection.active_field == Field::AuthType,
        false,
        LABEL_WIDTH,
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
        ));
        row_idx += 1;
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
        ));
    }

    lines.push(Line::from(""));
    let actions = vec![
        action_line(
            "Test connection",
            app.new_connection.active_field == Field::ActionTest,
        ),
        action_line(
            "Save connection",
            app.new_connection.active_field == Field::ActionSave,
        ),
    ];
    lines.extend(actions);

    if let Some(message) = &app.new_connection_feedback {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, layout[0]);
    render_input_cursor(
        frame,
        app,
        layout[0],
        user_row,
        host_row,
        auth_row,
        key_row,
        pass_row,
    );

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
    let footer = Paragraph::new(footer_lines).style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn draw_file_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.file_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Pick key file",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3), Constraint::Length(2)].as_ref())
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
                let prefix = if entry.is_dir { "[D]" } else { "[F]" };
                ListItem::new(format!("{prefix} {}", entry.name))
            })
            .collect()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    frame.render_stateful_widget(
        list,
        layout[1],
        &mut list_state(picker.selected, picker.entries.len()),
    );

    let footer = Paragraph::new("Enter to open/select, Backspace to up, Esc to cancel")
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[2]);
}

fn draw_key_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.key_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Pick recent key",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

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
        .highlight_symbol(">> ");
    frame.render_stateful_widget(
        list,
        inner,
        &mut list_state(picker.selected, picker.keys.len()),
    );
}

fn draw_master_password_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(60, 45, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Change master password",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let lines = vec![
        field_line(
            "Current",
            &app.master_change.current,
            app.master_change.active_field == MasterField::Current,
            true,
            LABEL_WIDTH,
        ),
        field_line(
            "New",
            &app.master_change.new_password,
            app.master_change.active_field == MasterField::New,
            true,
            LABEL_WIDTH,
        ),
        field_line(
            "Confirm",
            &app.master_change.confirm,
            app.master_change.active_field == MasterField::Confirm,
            true,
            LABEL_WIDTH,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to move, "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to save, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn draw_confirm_delete_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(50, 30, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            "Delete connection?",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

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
    let area = centered_rect(50, 25, frame.area());
    frame.render_widget(Clear, area);
    let title = if result.success { "Try success" } else { "Try failed" };
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

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
    let area = centered_rect(50, 25, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            notice.title.as_str(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(2)].as_ref())
        .split(inner);

    let message = Paragraph::new(notice.message.as_str()).wrap(Wrap { trim: true });
    frame.render_widget(message, layout[0]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("Press "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to close."),
    ]))
    .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, layout[1]);
}

fn field_line(
    label: &str,
    value: &str,
    active: bool,
    mask: bool,
    label_width: usize,
) -> Line<'static> {
    let display = if mask && !value.is_empty() {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let indicator = "> ";
    let indicator_style = if active {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
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

fn action_line(label: &str, active: bool) -> Line<'static> {
    let indicator = "> ";
    let indicator_style = if active {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
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
    user_row: Option<usize>,
    host_row: Option<usize>,
    _auth_row: Option<usize>,
    key_row: Option<usize>,
    pass_row: Option<usize>,
) {
    let (row, col) = match app.new_connection.active_field {
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

fn list_state(selected: usize, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(selected.min(len.saturating_sub(1))));
    }
    state
}
