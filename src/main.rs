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

const TICK_RATE: Duration = Duration::from_millis(150);

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
        let (_, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let details_height = if app.header_mode != crate::app::HeaderMode::Off {
            rows.saturating_sub(3)
        } else {
            rows
        };
        app.set_details_height(details_height);

        terminal.draw(|frame| ui::draw_ui(frame, &app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c'))
                    {
                        return Ok(());
                    }
                    if app.handle_key(key)? {
                        return Ok(());
                    }
                }
                Event::Mouse(_) => {}
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = std::time::Instant::now();
        }

        app.poll_remote_fetch();
        app.poll_transfer_progress();

        if let Some(action) = app.pending_action.take() {
            match action {
                AppAction::OpenTerminal => {
                    app.handle_terminal_mode(terminal)?;
                }
            }
        }
    }
}
