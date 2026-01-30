use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod app;
mod model;
mod ssh;
mod storage;
mod ui;

use app::App;
use model::AppAction;
use ui::constants::{HEADER_HEIGHT, TERMINAL_FOOTER_HEIGHT};

const TICK_RATE: Duration = Duration::from_millis(33);

fn main() -> Result<()> {
    let mut app = App::load_with_master()?;

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut last_tick = std::time::Instant::now();

    loop {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let tabs_offset = if app.terminal_tabs_open() {
            HEADER_HEIGHT.saturating_add(TERMINAL_FOOTER_HEIGHT)
        } else {
            0
        };
        let usable_rows = rows.saturating_sub(tabs_offset);
        let details_height = if app.header_mode != app::HeaderMode::Off {
            usable_rows.saturating_sub(3)
        } else {
            usable_rows
        };
        app.set_details_height(details_height);
        if app.terminal_tabs_open() {
            app.update_terminal_sizes(cols, usable_rows);
        }

        terminal.draw(|frame| ui::draw_ui(frame, &app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    let ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c'));
                    let ctrl_shift_c = ctrl_c && key.modifiers.contains(KeyModifiers::SHIFT);
                    if ctrl_c
                        && !ctrl_shift_c
                        && !(app.terminal_tabs_open() && app.active_terminal_tab > 0)
                    {
                        return Ok(());
                    }
                    if app.handle_key(key)? {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                    app.handle_terminal_mouse(mouse, cols, rows);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = std::time::Instant::now();
        }

        app.poll_remote_fetch();
        app.poll_transfer_progress();
        app.poll_terminal_output();
        app.poll_size_calc();

        if let Some(action) = app.pending_action.take() {
            match action {
                AppAction::OpenTerminal => {
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                    app.open_terminal_tab(
                        cols,
                        rows.saturating_sub(HEADER_HEIGHT + TERMINAL_FOOTER_HEIGHT),
                    )?;
                }
            }
        }
    }
}
