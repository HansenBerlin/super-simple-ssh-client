use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::app::App;
use crate::model::TransferDirection;
use crate::ui::constants::{
    KEY_PICKER_HEIGHT, KEY_PICKER_WIDTH, PICKER_FOOTER_HEIGHT, TRANSFER_PICKER_HEIGHT,
    TRANSFER_PICKER_WIDTH,
};
use crate::ui::helpers::{centered_rect, draw_popup_frame, list_state};

pub(crate) fn draw_file_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.file_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(TRANSFER_PICKER_WIDTH, TRANSFER_PICKER_HEIGHT, frame.area());
    let (title, border_style, footer_text) = if let Some(transfer) = &app.transfer {
        match (transfer.direction, transfer.step) {
            (TransferDirection::Upload, crate::model::TransferStep::PickSource) => (
                "Pick file or folder on THIS host to upload",
                Style::default().fg(Color::White),
                "Enter to open, S to select folder, Backspace to up, Esc to cancel",
            ),
            (TransferDirection::Download, crate::model::TransferStep::PickTarget) => (
                "Pick target folder",
                Style::default().fg(Color::White),
                "Enter to open, S to select folder, B to go back to source, Backspace to up, Esc to cancel",
            ),
            _ => (
                "Pick key file",
                Style::default(),
                "Enter to open/select, Backspace to up, Esc to cancel",
            ),
        }
    } else {
        (
            "Pick key file",
            Style::default(),
            "Enter to open/select, Backspace to up, Esc to cancel",
        )
    };
    let inner = draw_popup_frame(frame, area, title, border_style);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(PICKER_FOOTER_HEIGHT),
            ]
            .as_ref(),
        )
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
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled("> ", Style::default().fg(Color::White)));
    frame.render_stateful_widget(
        list,
        layout[1],
        &mut list_state(picker.selected, picker.entries.len()),
    );

    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::TOP));
    frame.render_widget(footer, layout[2]);
}

pub(crate) fn draw_key_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.key_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(KEY_PICKER_WIDTH, KEY_PICKER_HEIGHT, frame.area());
    let inner = draw_popup_frame(
        frame,
        area,
        "Pick recent key",
        Style::default().fg(Color::Yellow),
    );

    let items: Vec<ListItem> = picker
        .keys
        .iter()
        .map(|entry| {
            let suffix = if entry.password.is_some() { " (pw)" } else { "" };
            ListItem::new(format!("{}{}", entry.path, suffix))
        })
        .collect();

    let list = List::new(items)
        .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled("> ", Style::default().fg(Color::White)));
    frame.render_stateful_widget(
        list,
        inner,
        &mut list_state(picker.selected, picker.keys.len()),
    );
}

pub(crate) fn draw_remote_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let picker = match &app.remote_picker {
        Some(picker) => picker,
        None => return,
    };
    let area = centered_rect(TRANSFER_PICKER_WIDTH, TRANSFER_PICKER_HEIGHT, frame.area());
    let (title, border_style, footer_text) = if let Some(transfer) = &app.transfer {
        match (transfer.direction, transfer.step) {
            (TransferDirection::Download, crate::model::TransferStep::PickSource) => (
                "Pick remote source",
                Style::default().fg(Color::Green),
                "Enter to open, S to select folder, Backspace to up, Esc to cancel",
            ),
            (TransferDirection::Upload, crate::model::TransferStep::PickTarget) => (
                "Pick where to save the file or folder on the REMOTE host",
                Style::default().fg(Color::Green),
                "Enter to open, S to select folder, B to go back to source, Backspace to up, Esc to cancel",
            ),
            _ => (
                "Pick remote target",
                Style::default(),
                "Enter to open, S to select folder, Backspace to up, Esc to cancel",
            ),
        }
    } else {
        (
            "Pick remote target",
            Style::default(),
            "Enter to open, S to select folder, Backspace to up, Esc to cancel",
        )
    };
    let inner = draw_popup_frame(frame, area, title, border_style);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(PICKER_FOOTER_HEIGHT),
            ]
            .as_ref(),
        )
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
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol(Span::styled("> ", Style::default().fg(Color::White)));
        frame.render_stateful_widget(
            list,
            layout[1],
            &mut list_state(picker.selected, picker.entries.len()),
        );
    }

    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::TOP));
    frame.render_widget(footer, layout[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::model::{FileEntry, FilePickerState, RemoteEntry, RemotePickerState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn draw_file_picker_modal_smoke() {
        let mut app = App::for_test();
        app.file_picker = Some(FilePickerState {
            cwd: std::env::temp_dir(),
            entries: vec![
                FileEntry {
                    name: "a.txt".to_string(),
                    path: std::env::temp_dir().join("a.txt"),
                    is_dir: false,
                },
                FileEntry {
                    name: "dir".to_string(),
                    path: std::env::temp_dir().join("dir"),
                    is_dir: true,
                },
            ],
            selected: 0,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_file_picker_modal(frame, &app))
            .unwrap();
    }

    #[test]
    fn draw_remote_picker_modal_smoke() {
        let mut app = App::for_test();
        app.remote_picker = Some(RemotePickerState {
            cwd: "/".to_string(),
            entries: vec![RemoteEntry {
                name: "etc".to_string(),
                path: "/etc".to_string(),
                is_dir: true,
            }],
            selected: 0,
            loading: false,
            error: None,
            only_dirs: true,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_remote_picker_modal(frame, &app))
            .unwrap();
    }
}
