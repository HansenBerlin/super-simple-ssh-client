use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};

use crate::app::{App, HeaderMode};
use crate::model::AuthConfig;
use crate::ui::constants::{HEADER_HEIGHT, HELP_TEXT};

pub(crate) fn draw_saved_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
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
        .highlight_symbol(Span::styled(">", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
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
        let relative_index = app.selected_saved.saturating_sub(start);
        state.select(Some(relative_index));
    }
    frame.render_stateful_widget(list, list_area, &mut state);

    let selected_connected = app
        .connections
        .get(app.selected_saved)
        .map(|conn| connected.contains(&crate::model::connection_key(conn)))
        .unwrap_or(false);
    let connection_commands = if selected_connected {
        "(n)ew | (e)dit | (c)ancel | (x)delete"
    } else {
        "(n)ew | (e)dit | (c)onnect | (x)delete"
    };
    let commands = Paragraph::new(connection_commands)
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

pub(crate) fn draw_app_header(frame: &mut Frame<'_>, area: Rect) {
    let title = Paragraph::new("SUPER SIMPLE SSH 0.1.2")
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    frame.render_widget(title, area);
}

pub(crate) fn draw_help_header(frame: &mut Frame<'_>, area: Rect) {
    let help = Paragraph::new(HELP_TEXT)
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    "Help",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::ALL),
        )
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(help, area);
}

pub(crate) fn draw_open_tabs(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    logs_area: Option<Rect>,
    has_header: bool,
    render_header: bool,
) {
    let header_style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD);
    let tabs_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: HEADER_HEIGHT,
    };
    let (body_area, help_area) = if has_header {
        (
            Rect {
                x: area.x,
                y: area.y + HEADER_HEIGHT,
                width: area.width,
                height: area.height.saturating_sub(HEADER_HEIGHT),
            },
            Some(tabs_area),
        )
    } else {
        (area, None)
    };
    if render_header {
        if let Some(help_area) = help_area {
            match app.header_mode {
                HeaderMode::Help => {
                    let help = Paragraph::new(HELP_TEXT)
                        .block(
                            Block::default()
                                .title(Line::from(Span::styled(
                                    "Help",
                                    Style::default()
                                        .fg(Color::Magenta)
                                        .add_modifier(Modifier::BOLD),
                                )))
                                .borders(Borders::ALL),
                        )
                        .style(Style::default().fg(Color::Gray));
                    frame.render_widget(help, help_area);
                }
                HeaderMode::Logs => {
                    let log_lines = app
                        .log_lines
                        .iter()
                        .rev()
                        .take(help_area.height.saturating_sub(2) as usize)
                        .cloned()
                        .collect::<Vec<_>>();
                    let logs = Paragraph::new(log_lines.join("\n"))
                        .block(
                            Block::default()
                                .title(Line::from(Span::styled(
                                    "Logs",
                                    Style::default()
                                        .fg(Color::Magenta)
                                        .add_modifier(Modifier::BOLD),
                                )))
                                .borders(Borders::ALL),
                        )
                        .style(Style::default().fg(Color::Gray))
                        .wrap(Wrap { trim: true });
                    frame.render_widget(logs, help_area);
                }
                HeaderMode::Off => {}
            }
        }
    }

    let connected: HashSet<String> = app
        .open_connections
        .iter()
        .map(|conn| crate::model::connection_key(&conn.config))
        .collect();
    let details = if let Some(conn) = app.connections.get(app.selected_saved) {
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
        let has_error = app.last_error.contains_key(&key);
        let (start, end) = app.history_range(history_len, has_error);
        let page_size = app.history_page_size(has_error).max(1);
        let max_page = if history_len == 0 {
            0
        } else {
            history_len.saturating_sub(1) / page_size
        };
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
                    .title(Line::from(Span::styled("Connection details", header_style))),
            )
            .wrap(Wrap { trim: true })
    } else {
        Paragraph::new("No saved connection selected")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Line::from(Span::styled("Connection details", header_style))),
            )
            .alignment(Alignment::Center)
    };
    frame.render_widget(details, body_area);

    if let Some(logs_area) = logs_area {
        let log_lines = app
            .log_lines
            .iter()
            .rev()
            .take(logs_area.height.saturating_sub(2) as usize)
            .cloned()
            .collect::<Vec<_>>();
        let logs = Paragraph::new(log_lines.join("\n"))
            .block(
                Block::default()
                    .title(Line::from(Span::styled(
                        "Logs",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    )))
                    .borders(Borders::ALL),
            )
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        frame.render_widget(logs, logs_area);
    }
}

pub(crate) fn draw_terminal_tab_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut titles = Vec::with_capacity(app.terminal_tabs.len() + 1);
    titles.push(Line::from(Span::styled(
        "Connections",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for tab in &app.terminal_tabs {
        titles.push(Line::from(Span::raw(tab.title.clone())));
    }
    let tabs = Tabs::new(titles)
        .select(app.active_terminal_tab)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

pub(crate) fn draw_terminal_view(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let index = app.active_terminal_tab.saturating_sub(1);
    let Some(tab) = app.terminal_tabs.get(index) else {
        return;
    };
    let screen = tab.parser.screen();
    let contents = screen.contents().to_string();
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    let terminal = Paragraph::new(contents)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(terminal, area);
    let (row, col) = screen.cursor_position();
    if row < inner.height && col < inner.width {
        frame.set_cursor_position((inner.x + col, inner.y + row));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::app::App;
    use crate::model::{AuthConfig, ConnectionConfig};

    #[test]
    fn draw_terminal_footer_renders_keys() {
        let backend = TestBackend::new(60, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_terminal_footer(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("F6"));
        assert!(content.contains("F7"));
        assert!(content.contains("F8"));
    }

    #[test]
    fn draw_saved_list_renders_commands() {
        let mut app = App::for_test();
        app.connections.push(ConnectionConfig {
            name: "Test".to_string(),
            user: "u".to_string(),
            host: "h".to_string(),
            auth: AuthConfig::Password {
                password: "pw".to_string(),
            },
            history: vec![],
            last_remote_dir: None,
        });
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_saved_list(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("(n)ew"));
        assert!(content.contains("(e)dit"));
    }
}

pub(crate) fn draw_terminal_footer(frame: &mut Frame<'_>, area: Rect) {
    let footer = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("F6", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" previous tab | "),
            Span::styled("F7", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" next tab | "),
            Span::styled("F8", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" close tab"),
        ]),
    ])
    .style(Style::default().fg(Color::Gray))
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true });
    frame.render_widget(footer, area);
}
