use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::App;
use crate::model::Mode;
use crate::ui::constants::{
    HEADER_HEIGHT, TERMINAL_FOOTER_HEIGHT, help_columns, compact_columns,
};
use crate::ui::modals::{
    draw_confirm_delete_modal, draw_master_password_modal, draw_new_connection_modal,
    draw_notice_modal, draw_transfer_confirm_modal, draw_try_result_modal,
};
use crate::ui::panels::{
    draw_app_header, draw_help_header, draw_open_tabs, draw_saved_list, draw_terminal_footer,
    draw_terminal_tab_bar, draw_terminal_view,
};
use crate::ui::pickers::{
    draw_file_picker_modal, draw_key_picker_modal, draw_remote_picker_modal,
};

pub(crate) mod constants;
mod helpers;
mod modals;
mod panels;
mod pickers;

pub(crate) fn draw_ui(frame: &mut Frame<'_>, app: &App) {
    if app.terminal_tabs_open() {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(HEADER_HEIGHT),
                    Constraint::Min(1),
                    Constraint::Length(TERMINAL_FOOTER_HEIGHT),
                ]
                .as_ref(),
            )
            .split(frame.area());
        draw_terminal_tab_bar(frame, app, layout[0]);
        if app.active_terminal_tab == 0 {
            draw_main_ui(frame, app, layout[1], false);
        } else {
            draw_terminal_view(frame, app, layout[1]);
        }
        draw_terminal_footer(frame, layout[2]);
    } else {
        draw_main_ui(frame, app, frame.area(), true);
    }

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
    if app
        .transfer
        .as_ref()
        .is_some_and(|t| matches!(t.step, crate::model::TransferStep::Confirm))
        || app
            .transfer
            .as_ref()
            .is_some_and(|t| matches!(t.step, crate::model::TransferStep::Transferring))
            && !app.transfer_hidden
    {
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

fn draw_main_ui(frame: &mut Frame<'_>, app: &App, area: Rect, show_help_header: bool) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1)].as_ref())
        .split(area);

    let body = if app.header_mode == crate::app::HeaderMode::Help {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints(help_columns().as_ref())
            .split(layout[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints(compact_columns().as_ref())
            .split(layout[0])
    };

    let left = if app.header_mode != crate::app::HeaderMode::Off {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(HEADER_HEIGHT), Constraint::Min(1)].as_ref())
            .split(body[0])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)].as_ref())
            .split(body[0])
    };

    if app.header_mode != crate::app::HeaderMode::Off {
        draw_app_header(frame, left[0]);
        draw_saved_list(frame, app, left[1]);
    } else {
        draw_saved_list(frame, app, left[0]);
    }

    if app.header_mode == crate::app::HeaderMode::Help && show_help_header {
        let help_header = Rect {
            x: body[1].x,
            y: body[1].y,
            width: body[1].width.saturating_add(body[2].width),
            height: HEADER_HEIGHT,
        };
        draw_help_header(frame, help_header);
        let logs_body = Rect {
            x: body[2].x,
            y: body[2].y + HEADER_HEIGHT,
            width: body[2].width,
            height: body[2].height.saturating_sub(HEADER_HEIGHT),
        };
        draw_open_tabs(frame, app, body[1], Some(logs_body), true, false);
    } else if app.header_mode == crate::app::HeaderMode::Help {
        let logs_body = Rect {
            x: body[2].x,
            y: body[2].y,
            width: body[2].width,
            height: body[2].height,
        };
        draw_open_tabs(frame, app, body[1], Some(logs_body), false, false);
    } else {
        draw_open_tabs(
            frame,
            app,
            body[1],
            None,
            app.header_mode != crate::app::HeaderMode::Off && show_help_header,
            show_help_header,
        );
    }
}
