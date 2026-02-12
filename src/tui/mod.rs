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

use crate::repo;

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

                // Handle pending edit: suspend TUI, open $EDITOR, restore TUI
                if let Some((page_id, tmp_path, version)) = app.pending_edit.take() {
                    // Read original content for change detection
                    let original = std::fs::read_to_string(&tmp_path).unwrap_or_default();

                    // Suspend TUI
                    execute!(io::stdout(), LeaveAlternateScreen).map_err(KbError::Io)?;
                    disable_raw_mode().map_err(KbError::Io)?;

                    // Resolve editor command
                    let editor = std::env::var("EDITOR")
                        .or_else(|_| std::env::var("VISUAL"))
                        .unwrap_or_else(|_| "vi".to_string());

                    // Spawn editor
                    let status = std::process::Command::new(&editor)
                        .arg(&tmp_path)
                        .status();

                    // Restore TUI
                    enable_raw_mode().map_err(KbError::Io)?;
                    execute!(io::stdout(), EnterAlternateScreen).map_err(KbError::Io)?;
                    terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
                        .map_err(KbError::Io)?;

                    // Process result
                    match status {
                        Ok(exit) if exit.success() => {
                            let new_content = std::fs::read_to_string(&tmp_path)
                                .unwrap_or_default();
                            if new_content != original {
                                match repo::update_page(
                                    conn, &page_id, None,
                                    Some(&new_content), None, Some(version),
                                ) {
                                    Ok(_) => {
                                        let _ = repo::add_label(conn, &page_id, "human-edited");
                                        app.load_items(conn)?;
                                    }
                                    Err(KbError::VersionConflict { expected, actual }) => {
                                        app.content_lines.clear();
                                        app.content_lines.push(format!(
                                            "Edit conflict: expected version {}, page is now version {}. Your changes were NOT saved.",
                                            expected, actual
                                        ));
                                        app.content_lines.push("Re-select the page and try again.".to_string());
                                    }
                                    Err(e) => {
                                        app.content_lines.clear();
                                        app.content_lines.push(format!("Error saving: {}", e));
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            // Editor exited with non-zero — discard changes
                        }
                        Err(e) => {
                            app.content_lines.clear();
                            app.content_lines.push(format!("Failed to launch editor '{}': {}", editor, e));
                        }
                    }

                    // Clean up temp file
                    let _ = std::fs::remove_file(&tmp_path);
                }

                // Handle pending label edit: suspend TUI, open $EDITOR, restore TUI
                if let Some((page_id, tmp_path, _version)) = app.pending_label_edit.take() {
                    // Read original content for change detection
                    let original = std::fs::read_to_string(&tmp_path).unwrap_or_default();

                    // Suspend TUI
                    execute!(io::stdout(), LeaveAlternateScreen).map_err(KbError::Io)?;
                    disable_raw_mode().map_err(KbError::Io)?;

                    // Resolve editor command
                    let editor = std::env::var("EDITOR")
                        .or_else(|_| std::env::var("VISUAL"))
                        .unwrap_or_else(|_| "vi".to_string());

                    // Spawn editor
                    let status = std::process::Command::new(&editor)
                        .arg(&tmp_path)
                        .status();

                    // Restore TUI
                    enable_raw_mode().map_err(KbError::Io)?;
                    execute!(io::stdout(), EnterAlternateScreen).map_err(KbError::Io)?;
                    terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
                        .map_err(KbError::Io)?;

                    // Process result
                    match status {
                        Ok(exit) if exit.success() => {
                            let new_content = std::fs::read_to_string(&tmp_path)
                                .unwrap_or_default();
                            if new_content != original {
                                // Parse labels: one per line, skip empty and comment lines
                                let new_labels: Vec<String> = new_content
                                    .lines()
                                    .map(|l| l.trim())
                                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                    .map(|l| l.to_string())
                                    .collect();
                                match repo::set_labels(conn, &page_id, &new_labels) {
                                    Ok(_) => {
                                        let _ = repo::add_label(conn, &page_id, "human-edited");
                                        app.load_items(conn)?;
                                    }
                                    Err(e) => {
                                        app.content_lines.clear();
                                        app.content_lines.push(format!("Error saving labels: {}", e));
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            // Editor exited with non-zero — discard changes
                        }
                        Err(e) => {
                            app.content_lines.clear();
                            app.content_lines.push(format!("Failed to launch editor '{}': {}", editor, e));
                        }
                    }

                    // Clean up temp file
                    let _ = std::fs::remove_file(&tmp_path);
                }

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
