use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;
use crate::model::{AuthKind, Field, MasterField};
use crate::ui::constants::{LABEL_WIDTH, POPUP_MIN_HEIGHT, POPUP_MIN_WIDTH};

pub(crate) fn field_line(
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
    let indicator_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
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

pub(crate) fn action_line(label: &str, active: bool) -> Line<'static> {
    let indicator = if active { "> " } else { "  " };
    let indicator_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let spans = vec![
        Span::styled(indicator, indicator_style),
        Span::styled(label.to_string(), Style::default().add_modifier(Modifier::BOLD)),
    ];
    Line::from(spans)
}

pub(crate) fn truncate_text(value: &str, max_width: usize) -> String {
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

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

pub(crate) fn render_input_cursor(
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

pub(crate) fn render_master_cursor(
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

pub(crate) fn auth_kind_label(kind: AuthKind) -> &'static str {
    match kind {
        AuthKind::PasswordOnly => "Password only",
        AuthKind::PrivateKey => "Private key",
        AuthKind::PrivateKeyWithPassword => "Private key + password",
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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

pub(crate) fn centered_rect_by_height(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    centered_rect_abs(width, height, area)
}

pub(crate) fn centered_rect_abs(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.max(POPUP_MIN_WIDTH).min(area.width);
    let height = height.max(POPUP_MIN_HEIGHT).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

pub(crate) fn padded_rect(area: Rect, pad: u16) -> Rect {
    Rect {
        x: area.x + pad,
        y: area.y + pad,
        width: area.width.saturating_sub(pad * 2),
        height: area.height.saturating_sub(pad * 2),
    }
}

pub(crate) fn modal_height(content_lines: usize, footer_lines: usize) -> u16 {
    let total = content_lines + footer_lines;
    (total as u16).saturating_add(2 + 2)
}

pub(crate) fn draw_popup_frame(frame: &mut Frame<'_>, area: Rect, title: &str, style: Style) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_style(style);
    frame.render_widget(block, area);
    let inner = padded_rect(area, 1);
    if inner.height < 2 {
        return inner;
    }
    let title_line = Paragraph::new(title)
        .alignment(Alignment::Center)
        .style(style.add_modifier(Modifier::BOLD));
    frame.render_widget(
        title_line,
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );
    let line = "-".repeat(inner.width as usize);
    let separator = Paragraph::new(line).style(style);
    frame.render_widget(
        separator,
        Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 1,
        },
    );
    Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(2),
    }
}

pub(crate) fn list_state(selected: usize, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(selected.min(len.saturating_sub(1))));
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_text_handles_edges() {
        assert_eq!(truncate_text("abc", 0), "");
        assert_eq!(truncate_text("abc", 2), "ab");
        assert_eq!(truncate_text("abcd", 3), "abc");
        assert_eq!(truncate_text("abcdef", 4), "a...");
        assert_eq!(truncate_text("short", 10), "short");
    }

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn auth_kind_label_matches_variants() {
        assert_eq!(auth_kind_label(AuthKind::PasswordOnly), "Password only");
        assert_eq!(auth_kind_label(AuthKind::PrivateKey), "Private key");
        assert_eq!(
            auth_kind_label(AuthKind::PrivateKeyWithPassword),
            "Private key + password"
        );
    }

    #[test]
    fn list_state_clamps_selection() {
        let state = list_state(5, 0);
        assert!(state.selected().is_none());
        let state = list_state(5, 3);
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn centered_rect_abs_clamps_to_area() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 5,
        };
        let rect = centered_rect_abs(100, 100, area);
        assert_eq!(rect.width, 10);
        assert_eq!(rect.height, 5);
        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
    }
}
