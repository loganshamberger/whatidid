use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::db::KbError;
use rusqlite::Connection;

use super::app::{App, Focus, Mode};

/// Semantic actions the TUI can perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    MoveDown,
    MoveUp,
    Select,
    GoBack,
    FocusContent,
    FocusList,
    JumpToTop,
    JumpToBottom,
    EnterSearch,
    SubmitSearch,
    CancelSearch,
    SearchInput(char),
    SearchBackspace,
    Edit,
    EditLabels,
    None,
}

/// Map a key event to a semantic action based on current mode and focus.
pub fn map_key(app: &App, key: KeyEvent) -> Action {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }

    match app.mode {
        Mode::Search => map_search_key(key),
        Mode::Normal => match app.focus {
            Focus::List => map_list_key(app, key),
            Focus::Content => map_content_key(app, key),
        },
    }
}

fn map_search_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Enter => Action::SubmitSearch,
        KeyCode::Esc => Action::CancelSearch,
        KeyCode::Backspace => Action::SearchBackspace,
        KeyCode::Char(c) => Action::SearchInput(c),
        _ => Action::None,
    }
}

fn map_list_key(app: &App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
        KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
        KeyCode::Enter => Action::Select,
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => Action::GoBack,
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => Action::FocusContent,
        KeyCode::Char('/') => Action::EnterSearch,
        KeyCode::Char('e') => Action::Edit,
        KeyCode::Char('L') => Action::EditLabels,
        KeyCode::Char('G') => Action::JumpToBottom,
        KeyCode::Char('g') => {
            if app.pending_g {
                Action::JumpToTop
            } else {
                Action::None // will set pending_g
            }
        }
        _ => Action::None,
    }
}

fn map_content_key(app: &App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
        KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => Action::FocusList,
        KeyCode::Tab => Action::FocusList,
        KeyCode::Char('/') => Action::EnterSearch,
        KeyCode::Char('e') => Action::Edit,
        KeyCode::Char('L') => Action::EditLabels,
        KeyCode::Char('G') => Action::JumpToBottom,
        KeyCode::Char('g') => {
            if app.pending_g {
                Action::JumpToTop
            } else {
                Action::None
            }
        }
        _ => Action::None,
    }
}

/// Apply an action to the app state, potentially querying the database.
pub fn apply_action(
    app: &mut App,
    action: Action,
    conn: &Connection,
    content_height: u16,
) -> Result<(), KbError> {
    match action {
        Action::Quit => {
            app.running = false;
        }
        Action::MoveDown => match app.focus {
            Focus::List => {
                app.move_cursor_down();
                app.update_content(conn)?;
            }
            Focus::Content => {
                app.scroll_content_down();
            }
        },
        Action::MoveUp => match app.focus {
            Focus::List => {
                app.move_cursor_up();
                app.update_content(conn)?;
            }
            Focus::Content => {
                app.scroll_content_up();
            }
        },
        Action::Select => {
            app.select(conn)?;
        }
        Action::GoBack => {
            app.go_back(conn)?;
        }
        Action::FocusContent => {
            if !app.items.is_empty() {
                app.focus = Focus::Content;
            }
        }
        Action::FocusList => {
            app.focus = Focus::List;
        }
        Action::JumpToTop => match app.focus {
            Focus::List => {
                app.jump_to_top();
                app.update_content(conn)?;
            }
            Focus::Content => {
                app.scroll_content_to_top();
            }
        },
        Action::JumpToBottom => match app.focus {
            Focus::List => {
                app.jump_to_bottom();
                app.update_content(conn)?;
            }
            Focus::Content => {
                app.scroll_content_to_bottom(content_height);
            }
        },
        Action::EnterSearch => {
            app.enter_search();
        }
        Action::SubmitSearch => {
            app.submit_search(conn)?;
        }
        Action::CancelSearch => {
            app.cancel_search();
        }
        Action::SearchInput(c) => {
            app.search_input.push(c);
        }
        Action::SearchBackspace => {
            app.search_input.pop();
        }
        Action::Edit => {
            if let Some(edit_info) = app.prepare_edit(conn)? {
                app.pending_edit = Some(edit_info);
            }
        }
        Action::EditLabels => {
            if let Some(edit_info) = app.prepare_edit_labels(conn)? {
                app.pending_label_edit = Some(edit_info);
            }
        }
        Action::None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_key_with_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_ctrl_c_always_quits() {
        let app = App::new();
        let key = make_key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(&app, key), Action::Quit);
    }

    #[test]
    fn test_normal_list_keys() {
        let app = App::new();
        assert_eq!(map_key(&app, make_key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('j'))), Action::MoveDown);
        assert_eq!(map_key(&app, make_key(KeyCode::Down)), Action::MoveDown);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('k'))), Action::MoveUp);
        assert_eq!(map_key(&app, make_key(KeyCode::Up)), Action::MoveUp);
        assert_eq!(map_key(&app, make_key(KeyCode::Enter)), Action::Select);
        assert_eq!(map_key(&app, make_key(KeyCode::Esc)), Action::GoBack);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('l'))), Action::FocusContent);
        assert_eq!(map_key(&app, make_key(KeyCode::Tab)), Action::FocusContent);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('/'))), Action::EnterSearch);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('G'))), Action::JumpToBottom);
    }

    #[test]
    fn test_normal_content_keys() {
        let mut app = App::new();
        app.focus = Focus::Content;
        assert_eq!(map_key(&app, make_key(KeyCode::Char('j'))), Action::MoveDown);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('k'))), Action::MoveUp);
        assert_eq!(map_key(&app, make_key(KeyCode::Esc)), Action::FocusList);
        assert_eq!(map_key(&app, make_key(KeyCode::Tab)), Action::FocusList);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn test_search_mode_keys() {
        let mut app = App::new();
        app.mode = Mode::Search;
        assert_eq!(map_key(&app, make_key(KeyCode::Enter)), Action::SubmitSearch);
        assert_eq!(map_key(&app, make_key(KeyCode::Esc)), Action::CancelSearch);
        assert_eq!(map_key(&app, make_key(KeyCode::Backspace)), Action::SearchBackspace);
        assert_eq!(map_key(&app, make_key(KeyCode::Char('a'))), Action::SearchInput('a'));
    }

    #[test]
    fn test_edit_key_mapping_list() {
        let app = App::new();
        assert_eq!(map_key(&app, make_key(KeyCode::Char('e'))), Action::Edit);
    }

    #[test]
    fn test_edit_key_mapping_content() {
        let mut app = App::new();
        app.focus = Focus::Content;
        assert_eq!(map_key(&app, make_key(KeyCode::Char('e'))), Action::Edit);
    }

    #[test]
    fn test_edit_labels_key_mapping_list() {
        let app = App::new();
        assert_eq!(map_key(&app, make_key(KeyCode::Char('L'))), Action::EditLabels);
    }

    #[test]
    fn test_edit_labels_key_mapping_content() {
        let mut app = App::new();
        app.focus = Focus::Content;
        assert_eq!(map_key(&app, make_key(KeyCode::Char('L'))), Action::EditLabels);
    }

    #[test]
    fn test_gg_sequence_list() {
        let mut app = App::new();
        // First g: pending_g is false, so map_key returns None
        let action = map_key(&app, make_key(KeyCode::Char('g')));
        assert_eq!(action, Action::None);

        // Set pending_g, then second g should JumpToTop
        app.pending_g = true;
        let action = map_key(&app, make_key(KeyCode::Char('g')));
        assert_eq!(action, Action::JumpToTop);
    }
}
