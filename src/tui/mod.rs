pub mod app;
pub mod event;
pub mod ui;

use std::io;
use std::panic;
use std::time::Instant;

use crossterm::event::{self as ct_event, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use rusqlite::Connection;

use crate::db::KbError;

use self::app::App;
use self::event::{map_key, apply_action, Action};

/// RAII guard that ensures the terminal is restored on drop (including panics).
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Entry point for the TUI browser. Called from main.rs on `browse` subcommand.
pub fn run_browse(conn: &Connection) -> Result<(), KbError> {
    // Install a panic hook that restores the terminal before printing the panic.
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        original_hook(info);
    }));

    let _guard = TerminalGuard::new().map_err(KbError::Io)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(KbError::Io)?;

    let mut app = App::new();
    app.load_initial(conn)?;

    const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    let mut last_refresh = Instant::now();

    // Main event loop
    loop {
        // Draw
        terminal
            .draw(|f| ui::draw(f, &app))
            .map_err(KbError::Io)?;

        if !app.running {
            break;
        }

        // Poll for events
        if ct_event::poll(std::time::Duration::from_millis(100)).map_err(KbError::Io)? {
            if let Event::Key(key) = ct_event::read().map_err(KbError::Io)? {
                // Only handle key press events (not release/repeat)
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                let action = map_key(&app, key);

                // Handle the 'gg' sequence: first 'g' sets pending, second triggers jump
                if key.code == crossterm::event::KeyCode::Char('g')
                    && app.mode == app::Mode::Normal
                    && action == Action::None
                    && !app.pending_g
                {
                    app.pending_g = true;
                    continue;
                }

                // Reset pending_g after any key (it was either consumed or interrupted)
                app.pending_g = false;

                let content_height = terminal.size().map_err(KbError::Io)?.height.saturating_sub(4);
                apply_action(&mut app, action, conn, content_height)?;
                last_refresh = Instant::now();
            }
        } else if last_refresh.elapsed() >= REFRESH_INTERVAL {
            // No key event and refresh interval elapsed — reload from database
            app.refresh(conn)?;
            last_refresh = Instant::now();
        }
    }

    Ok(())
}
