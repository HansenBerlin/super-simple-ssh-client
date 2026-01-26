use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};

use crate::app::App;
use crate::model::{AuthKind, Field, MasterField};
use crate::ui::constants::{
    LABEL_WIDTH, MODAL_MAX_HEIGHT_PERCENT, MODAL_MIN_WIDTH, MODAL_WIDTH_PERCENT,
    TRANSFER_CONFIRM_WIDTH_PERCENT,
};
use crate::ui::helpers::{
    action_line, auth_kind_label, centered_rect_abs, centered_rect_by_height, draw_popup_frame,
    field_line, format_bytes, modal_height, render_input_cursor, render_master_cursor,
};

pub(crate) fn draw_new_connection_modal(frame: &mut Frame<'_>, app: &App) {
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
    if matches!(
        app.new_connection.auth_kind,
        AuthKind::PrivateKey | AuthKind::PrivateKeyWithPassword
    ) {
        footer_lines.push(Line::from(Span::styled(
            "F2 to browse for key file | F3 to pick from recent keys",
            Style::default().fg(Color::Gray),
        )));
    }

    let area_width = (frame.area().width.saturating_mul(MODAL_WIDTH_PERCENT) / 100)
        .min(frame.area().width.saturating_sub(2))
        .max(MODAL_MIN_WIDTH);
    let pad = 1u16;
    let content_width = area_width.saturating_sub(2 + pad * 2);
    let value_width = content_width.saturating_sub(2 + LABEL_WIDTH as u16 + 2) as usize;
    let max_height = frame.area().height.saturating_mul(MODAL_MAX_HEIGHT_PERCENT) / 100;

    let title = if app.edit_index.is_some() {
        "Edit connection"
    } else {
        "New connection"
    };

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
    let desired_height = modal_height(content_lines, footer_lines.len() + 1);
    let area_height = desired_height
        .max(10)
        .min(max_height.max(10))
        .min(frame.area().height.saturating_sub(2));
    let area = centered_rect_abs(area_width, area_height, frame.area());
    let inner = draw_popup_frame(frame, area, title, Style::default().fg(Color::Yellow));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(1),
                Constraint::Length(footer_lines.len() as u16 + 1),
            ]
            .as_ref(),
        )
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

    let footer = Paragraph::new(footer_lines)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, layout[1]);
}

pub(crate) fn draw_transfer_confirm_modal(frame: &mut Frame<'_>, app: &App) {
    let transfer = match &app.transfer {
        Some(transfer) => transfer,
        None => return,
    };
    let transferring = transfer.step == crate::model::TransferStep::Transferring;
    let content_lines = if transferring { 4 } else { 3 };
    let height = modal_height(content_lines + 2, 2);
    let area = centered_rect_by_height(TRANSFER_CONFIRM_WIDTH_PERCENT, height, frame.area());
    let border_style = Style::default();
    let back_label = "to go back to target";
    let inner = draw_popup_frame(frame, area, "Confirm transfer", border_style);

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
    let size_label = if let Some(size) = transfer.size_bytes {
        format_bytes(size)
    } else if transfer.step == crate::model::TransferStep::Confirm {
        String::from("calculating size...")
    } else {
        String::from("-")
    };

    let mut lines = vec![
        Line::from(format!("Source: {source}")),
        Line::from(format!("Target: {target}")),
        Line::from(format!("Size: {size_label}")),
    ];
    if transferring {
        lines.push(Line::from("Transferring..."));
    }
    let layout = if transferring {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Min(3),
                    Constraint::Length(1),
                    Constraint::Length(2),
                ]
                .as_ref(),
            )
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(2)].as_ref())
            .split(inner)
    };
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, layout[0]);

    if transferring {
        let total = transfer.size_bytes.unwrap_or(0);
        let current = transfer.progress_bytes.min(total);
        let ratio = if total == 0 {
            0.0
        } else {
            current as f64 / total as f64
        };
        let label = if total == 0 {
            "0 B".to_string()
        } else {
            format!("{} / {}", format_bytes(current), format_bytes(total))
        };
        let gauge = Gauge::default()
            .ratio(ratio)
            .label(label)
            .style(Style::default().fg(Color::Gray))
            .gauge_style(Style::default().fg(Color::Green));
        frame.render_widget(gauge, layout[1]);
    }

    let footer = if transferring {
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to hide, "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]))
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to transfer, "),
            Span::styled("B", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::raw(back_label),
            Span::raw(", "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cancel"),
        ]))
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP))
    };
    let footer_area = if transferring { layout[2] } else { layout[1] };
    frame.render_widget(footer, footer_area);
}

pub(crate) fn draw_master_password_modal(frame: &mut Frame<'_>, app: &App) {
    let height = modal_height(6, 2);
    let area = centered_rect_by_height(60, height, frame.area());
    let inner = draw_popup_frame(
        frame,
        area,
        "Change master password",
        Style::default().fg(Color::Yellow),
    );
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)].as_ref())
        .split(inner);

    let value_width = layout[0]
        .width
        .saturating_sub(2 + LABEL_WIDTH as u16 + 2) as usize;
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
    render_master_cursor(frame, app, layout[0], current_row, new_row, confirm_row);

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
    .style(Style::default().fg(Color::Gray))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, layout[1]);
}

pub(crate) fn draw_confirm_delete_modal(frame: &mut Frame<'_>, app: &App) {
    let height = modal_height(1, 2);
    let area = centered_rect_by_height(50, height, frame.area());
    let inner = draw_popup_frame(
        frame,
        area,
        "Delete connection?",
        Style::default().fg(Color::Yellow),
    );

    let label = app
        .delete_index
        .and_then(|index| app.connections.get(index))
        .map(|conn| conn.label())
        .unwrap_or_else(|| "Unknown".to_string());

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)].as_ref())
        .split(inner);

    let message = Paragraph::new(format!("Delete {label}?"))
        .wrap(Wrap { trim: true });
    frame.render_widget(message, layout[0]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" or "),
        Span::styled("Y", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to confirm, "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" or "),
        Span::styled("N", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to cancel"),
    ]))
    .style(Style::default().fg(Color::Gray))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, layout[1]);
}

pub(crate) fn draw_try_result_modal(frame: &mut Frame<'_>, app: &App) {
    let result = match &app.try_result {
        Some(result) => result,
        None => return,
    };
    let height = modal_height(2, 2);
    let area = centered_rect_by_height(50, height, frame.area());
    let title = if result.success { "Try success" } else { "Try failed" };
    let inner = draw_popup_frame(frame, area, title, Style::default().fg(Color::Yellow));

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
    .style(Style::default().fg(Color::Gray))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, layout[1]);
}

pub(crate) fn draw_notice_modal(frame: &mut Frame<'_>, app: &App) {
    let notice = match &app.notice {
        Some(notice) => notice,
        None => return,
    };
    let message_lines = notice.message.lines().count().max(1);
    let footer_lines = if app.notice_action_label().is_some() { 1 } else { 1 };
    let height = modal_height(message_lines + footer_lines + 2, 1);
    let area = centered_rect_by_height(50, height, frame.area());
    let inner = draw_popup_frame(
        frame,
        area,
        notice.title.as_str(),
        Style::default().fg(Color::Yellow),
    );

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
        .block(Block::default().borders(Borders::TOP))
    } else {
        Paragraph::new(Line::from(vec![
            Span::raw("Press "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to close."),
        ]))
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP))
    };
    frame.render_widget(footer, layout[1]);
}
