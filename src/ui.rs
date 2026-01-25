use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::model::{AuthConfig, AuthKind, Field, MasterField, Mode};

const HELP_TEXT: &str =
    "n = new | e = edit | c = connect | d = disconnect | t = terminal | m = master pw | x = delete | q = quit";

pub(crate) fn draw_ui(frame: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1)].as_ref())
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(layout[0]);

    draw_saved_list(frame, app, body[0]);
    draw_open_tabs(frame, app, body[1]);

    if app.mode == Mode::NewConnection {
        draw_new_connection_modal(frame, app);
        if app.file_picker.is_some() {
            draw_file_picker_modal(frame, app);
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
    let connected: HashSet<String> = app
        .open_connections
        .iter()
        .map(|conn| crate::model::connection_key(&conn.config))
        .collect();
    let items: Vec<ListItem> = if app.connections.is_empty() {
        vec![ListItem::new("No saved connections")]
    } else {
        app.connections
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
                    "C "
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
        .block(
            Block::default()
                .title("Available connections")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    frame.render_stateful_widget(
        list,
        area,
        &mut list_state(app.selected_saved, app.connections.len()),
    );
}

fn draw_open_tabs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let tabs_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 3,
    };
    let body_area = Rect {
        x: area.x,
        y: area.y + 3,
        width: area.width,
        height: area.height.saturating_sub(3),
    };

    let help = Paragraph::new(HELP_TEXT)
        .block(Block::default().title("Help").borders(Borders::ALL))
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(help, tabs_area);

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

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Past connections:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if conn.history.is_empty() {
            lines.push(Line::from("  (none)"));
        } else {
            for entry in conn.history.iter().rev().take(5) {
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
                    .title("Connection details"),
            )
            .wrap(Wrap { trim: true })
    } else {
        Paragraph::new("No saved connection selected")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Connection details"),
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
    let block = Block::default().title(title).borders(Borders::ALL);
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(4),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(2),
            ]
            .as_ref(),
        )
        .split(inner);

    let mut lines = Vec::new();
    lines.push(field_line(
        "User",
        &app.new_connection.user,
        app.new_connection.active_field == Field::User,
        false,
    ));
    lines.push(field_line(
        "Host",
        &app.new_connection.host,
        app.new_connection.active_field == Field::Host,
        false,
    ));
    lines.push(field_line(
        "Auth",
        auth_kind_label(app.new_connection.auth_kind),
        app.new_connection.active_field == Field::AuthType,
        false,
    ));
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PrivateKey | AuthKind::PrivateKeyWithPassword
    ) {
        lines.push(field_line(
            "Key path",
            &app.new_connection.key_path,
            app.new_connection.active_field == Field::KeyPath,
            false,
        ));
        lines.push(Line::from(Span::styled(
            "F2 to browse for key file",
            Style::default().fg(Color::Gray),
        )));
    }
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PasswordOnly | AuthKind::PrivateKeyWithPassword
    ) {
        lines.push(field_line(
            "Password",
            &app.new_connection.password,
            app.new_connection.active_field == Field::Password,
            true,
        ));
    }

    if let Some(message) = &app.new_connection_feedback {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, layout[0]);

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
    let actions_paragraph = Paragraph::new(actions).wrap(Wrap { trim: true });
    frame.render_widget(actions_paragraph, layout[1]);

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
    frame.render_widget(footer, layout[2]);
}

fn draw_file_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.file_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Pick key file")
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

fn draw_master_password_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(60, 45, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title("Change master password")
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
        ),
        field_line(
            "New",
            &app.master_change.new_password,
            app.master_change.active_field == MasterField::New,
            true,
        ),
        field_line(
            "Confirm",
            &app.master_change.confirm,
            app.master_change.active_field == MasterField::Confirm,
            true,
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
        .title("Delete connection?")
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
    let block = Block::default().title(title).borders(Borders::ALL);
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
    let block = Block::default().title(notice.title.as_str()).borders(Borders::ALL);
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

fn field_line(label: &str, value: &str, active: bool, mask: bool) -> Line<'static> {
    let display = if mask && !value.is_empty() {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let indicator = if active { "> " } else { "  " };
    let spans = vec![
        Span::styled(indicator, Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{label}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(display),
    ];
    Line::from(spans)
}

fn action_line(label: &str, active: bool) -> Line<'static> {
    let indicator = if active { "> " } else { "  " };
    let spans = vec![
        Span::styled(indicator, Style::default().fg(Color::Yellow)),
        Span::styled(label.to_string(), Style::default().add_modifier(Modifier::BOLD)),
    ];
    Line::from(spans)
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
