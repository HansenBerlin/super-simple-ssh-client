use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::{backend::CrosstermBackend};

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
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut last_tick = std::time::Instant::now();

    loop {
        terminal.draw(|frame| ui::draw_ui(frame, &app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c'))
                {
                    return Ok(());
                }
                if app.handle_key(key)? {
                    return Ok(());
                }
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = std::time::Instant::now();
        }

        if let Some(action) = app.pending_action.take() {
            match action {
                AppAction::OpenTerminal => {
                    app.handle_terminal_mode(terminal)?;
                }
            }
        }
    }
}
