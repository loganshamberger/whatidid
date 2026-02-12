use crate::db::KbError;
use crate::models::{Link, Page, SearchResult, Space};
use crate::{repo, search};
use rusqlite::Connection;

/// Which pane has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Content,
}

/// Input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
}

/// What the left pane is currently showing.
#[derive(Debug, Clone)]
pub enum NavState {
    SpaceList,
    PageList {
        space: Space,
    },
    ChildPageList {
        space: Space,
        parent: Box<Page>,
    },
    SearchResults {
        query: String,
        /// The state to return to when pressing Esc.
        previous: Box<NavState>,
    },
}

/// Display item in the left pane list.
#[derive(Debug, Clone)]
pub enum ListItem {
    Space(Space),
    Page { page: Page, expandable: bool },
    SearchResult(SearchResult),
}

impl ListItem {
    pub fn display_text(&self) -> String {
        match self {
            ListItem::Space(s) => format!("{} ({})", s.name, s.slug),
            ListItem::Page { page, expandable } => {
                let prefix = if *expandable { "[+] " } else { "    " };
                format!("{}{}", prefix, page.title)
            }
            ListItem::SearchResult(r) => {
                format!("{} [{}]", r.page.title, r.page.page_type)
            }
        }
    }
}

/// Info needed to perform an edit after suspending the TUI.
/// Holds (page_id, temp_file_path, page_version).
pub type PendingEdit = (String, std::path::PathBuf, i64);

/// The main application state for the TUI browser.
pub struct App {
    pub running: bool,
    pub focus: Focus,
    pub mode: Mode,
    pub nav_state: NavState,
    pub items: Vec<ListItem>,
    pub cursor: usize,
    pub content_scroll: u16,
    pub search_input: String,
    /// Cached right-pane content lines for scrolling.
    pub content_lines: Vec<String>,
    /// Links for the currently selected page.
    pub links: Vec<Link>,
    /// True if waiting for second 'g' in gg sequence.
    pub pending_g: bool,
    /// Set when user presses 'e' — the event loop suspends TUI to open $EDITOR.
    pub pending_edit: Option<PendingEdit>,
    /// Set when user presses 'L' — the event loop suspends TUI to edit labels in $EDITOR.
    pub pending_label_edit: Option<PendingEdit>,
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            focus: Focus::List,
            mode: Mode::Normal,
            nav_state: NavState::SpaceList,
            items: Vec::new(),
            cursor: 0,
            content_scroll: 0,
            search_input: String::new(),
            content_lines: Vec::new(),
            links: Vec::new(),
            pending_g: false,
            pending_edit: None,
            pending_label_edit: None,
        }
    }

    /// Load the initial data (space list) from the database.
    pub fn load_initial(&mut self, conn: &Connection) -> Result<(), KbError> {
        self.nav_state = NavState::SpaceList;
        self.load_items(conn)
    }

    /// Refresh items from the database, preserving cursor and scroll position.
    /// Used for periodic auto-refresh to pick up external changes.
    pub fn refresh(&mut self, conn: &Connection) -> Result<(), KbError> {
        let prev_scroll = self.content_scroll;
        self.load_items(conn)?;
        // Restore scroll position, clamped to new content length
        self.content_scroll = prev_scroll.min(self.content_lines.len().saturating_sub(1) as u16);
        Ok(())
    }

    /// Reload items for the current nav state.
    pub fn load_items(&mut self, conn: &Connection) -> Result<(), KbError> {
        self.items = match &self.nav_state {
            NavState::SpaceList => {
                let spaces = repo::list_spaces(conn)?;
                spaces.into_iter().map(ListItem::Space).collect()
            }
            NavState::PageList { space } => {
                let pages = repo::list_top_level_pages(conn, &space.id)?;
                self.pages_to_items(conn, pages)?
            }
            NavState::ChildPageList { parent, .. } => {
                let pages = repo::list_child_pages(conn, &parent.id)?;
                self.pages_to_items(conn, pages)?
            }
            NavState::SearchResults { query, .. } => {
                let params = search::SearchParams {
                    query: Some(query.clone()),
                    space_id: None,
                    page_type: None,
                    label: None,
                    created_by_agent: None,
                    section: None,
                };
                let results = search::search_pages(conn, &params)?;
                results.into_iter().map(ListItem::SearchResult).collect()
            }
        };

        // Clamp cursor
        if self.items.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.items.len() {
            self.cursor = self.items.len() - 1;
        }

        self.content_scroll = 0;
        self.update_content(conn)?;
        Ok(())
    }

    fn pages_to_items(&self, conn: &Connection, pages: Vec<Page>) -> Result<Vec<ListItem>, KbError> {
        let mut items = Vec::new();
        for page in pages {
            let expandable = repo::has_children(conn, &page.id)?;
            items.push(ListItem::Page { page, expandable });
        }
        Ok(items)
    }

    /// Update the right pane content based on the currently selected item.
    pub fn update_content(&mut self, conn: &Connection) -> Result<(), KbError> {
        self.content_lines.clear();
        self.links.clear();
        self.content_scroll = 0;

        if self.items.is_empty() {
            self.content_lines.push("(empty)".to_string());
            return Ok(());
        }

        // Clone the selected item to avoid borrow conflict with &mut self
        let item = self.items[self.cursor].clone();

        match &item {
            ListItem::Space(s) => {
                self.content_lines.push(format!("Space:   {}", s.name));
                self.content_lines.push(format!("Slug:    {}", s.slug));
                self.content_lines.push(format!("ID:      {}", s.id));
                self.content_lines.push(format!("Created: {}", s.created_at));
                self.content_lines.push(format!("Updated: {}", s.updated_at));
                if !s.description.is_empty() {
                    self.content_lines.push(String::new());
                    self.content_lines.push(s.description.clone());
                }
            }
            ListItem::Page { page, .. } => {
                self.build_page_content(conn, page)?;
            }
            ListItem::SearchResult(r) => {
                self.build_page_content(conn, &r.page)?;
                if !r.excerpt.is_empty() {
                    self.content_lines.push(String::new());
                    self.content_lines.push("--- Match ---".to_string());
                    self.content_lines.push(r.excerpt.clone());
                }
            }
        }
        Ok(())
    }

    fn build_page_content(&mut self, conn: &Connection, page: &Page) -> Result<(), KbError> {
        self.content_lines.push(format!("Title:   {}", page.title));
        self.content_lines.push(format!("Type:    {}", page.page_type));
        self.content_lines.push(format!("ID:      {}", page.id));
        if !page.labels.is_empty() {
            self.content_lines.push(format!("Labels:  {}", page.labels.join(", ")));
        }
        self.content_lines.push(format!("Author:  {} / {}", page.created_by_user, page.created_by_agent));
        self.content_lines.push(format!("Created: {}", page.created_at));
        self.content_lines.push(format!("Updated: {}", page.updated_at));
        self.content_lines.push(format!("Version: {}", page.version));
        self.content_lines.push(String::new());

        // Render sections if available, otherwise raw content
        if let Some(ref sections) = page.sections {
            if let Some(obj) = sections.as_object() {
                let ordered_keys: Vec<(&str, &str)> = if let Some(schema) = page.page_type.section_schema() {
                    schema.iter().map(|d| (d.key, d.name)).collect()
                } else {
                    let mut keys: Vec<&String> = obj.keys().collect();
                    keys.sort();
                    keys.into_iter().map(|k| (k.as_str(), k.as_str())).collect()
                };

                let mut first = true;
                for (key, display_name) in &ordered_keys {
                    if let Some(val) = obj.get(*key) {
                        if let Some(text) = val.as_str() {
                            if !first {
                                self.content_lines.push(String::new());
                            }
                            first = false;
                            self.content_lines.push(format!("--- {} ---", display_name));
                            for line in text.lines() {
                                self.content_lines.push(line.to_string());
                            }
                        }
                    }
                }
                // Extra keys not in schema
                if let Some(schema) = page.page_type.section_schema() {
                    for (key, val) in obj {
                        if !schema.iter().any(|d| d.key == key) {
                            if let Some(text) = val.as_str() {
                                if !first {
                                    self.content_lines.push(String::new());
                                }
                                first = false;
                                self.content_lines.push(format!("--- {} ---", key));
                                for line in text.lines() {
                                    self.content_lines.push(line.to_string());
                                }
                            }
                        }
                    }
                }
            } else {
                for line in page.content.lines() {
                    self.content_lines.push(line.to_string());
                }
            }
        } else {
            for line in page.content.lines() {
                self.content_lines.push(line.to_string());
            }
        }

        // Links
        let page_links = repo::list_links(conn, &page.id)?;
        if !page_links.is_empty() {
            self.content_lines.push(String::new());
            self.content_lines.push("--- Links ---".to_string());
            for link in &page_links {
                if link.source_id == page.id {
                    self.content_lines.push(format!("  {} -> {}", link.relation, link.target_id));
                } else {
                    self.content_lines.push(format!("  {} <- {}", link.relation, link.source_id));
                }
            }
            self.links = page_links;
        }

        Ok(())
    }

    // === Navigation ===

    pub fn move_cursor_down(&mut self) {
        if !self.items.is_empty() && self.cursor < self.items.len() - 1 {
            self.cursor += 1;
        }
    }

    pub fn move_cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn jump_to_top(&mut self) {
        self.cursor = 0;
    }

    pub fn jump_to_bottom(&mut self) {
        if !self.items.is_empty() {
            self.cursor = self.items.len() - 1;
        }
    }

    pub fn scroll_content_down(&mut self) {
        self.content_scroll = self.content_scroll.saturating_add(1);
    }

    pub fn scroll_content_up(&mut self) {
        self.content_scroll = self.content_scroll.saturating_sub(1);
    }

    pub fn scroll_content_to_top(&mut self) {
        self.content_scroll = 0;
    }

    pub fn scroll_content_to_bottom(&mut self, visible_height: u16) {
        let total = self.content_lines.len() as u16;
        if total > visible_height {
            self.content_scroll = total - visible_height;
        }
    }

    /// Select the current item (Enter key). Returns true if navigation changed.
    pub fn select(&mut self, conn: &Connection) -> Result<bool, KbError> {
        if self.items.is_empty() {
            return Ok(false);
        }

        match self.items[self.cursor].clone() {
            ListItem::Space(space) => {
                self.nav_state = NavState::PageList { space };
                self.cursor = 0;
                self.load_items(conn)?;
                Ok(true)
            }
            ListItem::Page { page, expandable } => {
                if expandable {
                    // Get the space from the current nav state
                    let space = self.current_space().cloned();
                    if let Some(space) = space {
                        self.nav_state = NavState::ChildPageList {
                            space,
                            parent: Box::new(page),
                        };
                        self.cursor = 0;
                        self.load_items(conn)?;
                        return Ok(true);
                    }
                }
                // Leaf page: focus the content pane
                self.focus = Focus::Content;
                Ok(true)
            }
            ListItem::SearchResult(r) => {
                // Focus the content pane for the selected search result
                let _ = r;
                self.focus = Focus::Content;
                Ok(true)
            }
        }
    }

    /// Go back one level (Esc key in list focus).
    pub fn go_back(&mut self, conn: &Connection) -> Result<(), KbError> {
        match &self.nav_state {
            NavState::SpaceList => {
                // Already at root — quit
                self.running = false;
            }
            NavState::PageList { .. } => {
                self.nav_state = NavState::SpaceList;
                self.cursor = 0;
                self.load_items(conn)?;
            }
            NavState::ChildPageList { space, parent } => {
                // Check if the parent itself has a parent
                if let Some(ref grandparent_id) = parent.parent_id {
                    let grandparent = repo::get_page(conn, grandparent_id)?;
                    self.nav_state = NavState::ChildPageList {
                        space: space.clone(),
                        parent: Box::new(grandparent),
                    };
                } else {
                    self.nav_state = NavState::PageList {
                        space: space.clone(),
                    };
                }
                self.cursor = 0;
                self.load_items(conn)?;
            }
            NavState::SearchResults { previous, .. } => {
                self.nav_state = *previous.clone();
                self.cursor = 0;
                self.load_items(conn)?;
            }
        }
        Ok(())
    }

    /// Enter search mode.
    pub fn enter_search(&mut self) {
        self.mode = Mode::Search;
        self.search_input.clear();
    }

    /// Submit the search query.
    pub fn submit_search(&mut self, conn: &Connection) -> Result<(), KbError> {
        self.mode = Mode::Normal;
        if self.search_input.is_empty() {
            return Ok(());
        }
        let query = self.search_input.clone();
        let previous = Box::new(self.nav_state.clone());
        self.nav_state = NavState::SearchResults { query, previous };
        self.cursor = 0;
        self.focus = Focus::List;
        self.load_items(conn)?;
        Ok(())
    }

    /// Cancel search mode.
    pub fn cancel_search(&mut self) {
        self.mode = Mode::Normal;
        self.search_input.clear();
    }

    /// Prepare to edit the currently selected page in $EDITOR.
    /// Returns None if the selection is a Space or the list is empty.
    /// On success, writes the page content to a temp file and returns the edit info.
    pub fn prepare_edit(&self, conn: &Connection) -> Result<Option<PendingEdit>, KbError> {
        if self.items.is_empty() {
            return Ok(None);
        }

        let page = match &self.items[self.cursor] {
            ListItem::Space(_) => return Ok(None),
            ListItem::Page { page, .. } => repo::get_page(conn, &page.id)?,
            ListItem::SearchResult(r) => repo::get_page(conn, &r.page.id)?,
        };

        let tmp_dir = std::env::temp_dir();
        let filename = format!("whatidid-{}.md", &page.id[..8.min(page.id.len())]);
        let tmp_path = tmp_dir.join(filename);
        std::fs::write(&tmp_path, &page.content).map_err(KbError::Io)?;

        Ok(Some((page.id, tmp_path, page.version)))
    }

    /// Prepare to edit labels for the currently selected page in $EDITOR.
    /// Returns None if the selection is a Space or the list is empty.
    /// On success, writes the current labels to a temp file and returns the edit info.
    pub fn prepare_edit_labels(&self, conn: &Connection) -> Result<Option<PendingEdit>, KbError> {
        if self.items.is_empty() {
            return Ok(None);
        }

        let page = match &self.items[self.cursor] {
            ListItem::Space(_) => return Ok(None),
            ListItem::Page { page, .. } => repo::get_page(conn, &page.id)?,
            ListItem::SearchResult(r) => repo::get_page(conn, &r.page.id)?,
        };

        let tmp_dir = std::env::temp_dir();
        let filename = format!("whatidid-labels-{}.md", &page.id[..8.min(page.id.len())]);
        let tmp_path = tmp_dir.join(filename);

        let mut content = format!("# Labels for: {}\n", page.title);
        content.push_str("# One label per line. Empty lines and lines starting with # are ignored.\n");
        for label in &page.labels {
            content.push_str(label);
            content.push('\n');
        }

        std::fs::write(&tmp_path, &content).map_err(KbError::Io)?;

        Ok(Some((page.id, tmp_path, page.version)))
    }

    fn current_space(&self) -> Option<&Space> {
        match &self.nav_state {
            NavState::PageList { space } => Some(space),
            NavState::ChildPageList { space, .. } => Some(space),
            _ => None,
        }
    }

    /// Returns the title for the left pane based on current nav state.
    pub fn left_pane_title(&self) -> String {
        match &self.nav_state {
            NavState::SpaceList => "Spaces".to_string(),
            NavState::PageList { space } => format!("{} / Pages", space.slug),
            NavState::ChildPageList { space, parent } => {
                format!("{} / {}", space.slug, parent.title)
            }
            NavState::SearchResults { query, .. } => format!("Search: {}", query),
        }
    }

    /// Returns the status line hint text.
    pub fn status_hint(&self) -> &'static str {
        match self.mode {
            Mode::Search => "Type query, Enter:submit, Esc:cancel",
            Mode::Normal => match self.focus {
                Focus::List => "j/k:nav  Enter:select  e:edit  L:labels  Esc:back  /:search  q:quit",
                Focus::Content => "j/k:scroll  e:edit  L:labels  h/Esc:back  /:search  q:quit",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        let sql = include_str!("../../migrations/001_initial.sql");
        conn.execute_batch(sql).expect("run migration 001");
        let sql2 = include_str!("../../migrations/002_sections.sql");
        conn.execute_batch(sql2).expect("run migration 002");
        let sql3 = include_str!("../../migrations/003_timestamps.sql");
        conn.execute_batch(sql3).expect("run migration 003");
        conn
    }

    #[test]
    fn test_app_initial_load_empty() {
        let conn = setup_test_db();
        let mut app = App::new();
        app.load_initial(&conn).expect("load initial");
        assert!(app.items.is_empty());
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_app_space_list() {
        let conn = setup_test_db();
        repo::create_space(&conn, "alpha", "Alpha", "").unwrap();
        repo::create_space(&conn, "beta", "Beta", "").unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        assert_eq!(app.items.len(), 2);
    }

    #[test]
    fn test_app_cursor_movement() {
        let conn = setup_test_db();
        repo::create_space(&conn, "a", "A", "").unwrap();
        repo::create_space(&conn, "b", "B", "").unwrap();
        repo::create_space(&conn, "c", "C", "").unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();

        assert_eq!(app.cursor, 0);
        app.move_cursor_down();
        assert_eq!(app.cursor, 1);
        app.move_cursor_down();
        assert_eq!(app.cursor, 2);
        app.move_cursor_down(); // at end, should not move
        assert_eq!(app.cursor, 2);
        app.move_cursor_up();
        assert_eq!(app.cursor, 1);
        app.jump_to_bottom();
        assert_eq!(app.cursor, 2);
        app.jump_to_top();
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_app_drill_into_space() {
        let conn = setup_test_db();
        let space = repo::create_space(&conn, "s", "S", "").unwrap();
        repo::create_page(
            &conn, &space.id, None, "Page1", crate::models::PageType::Reference,
            "content", None, &[], "u", "a",
        ).unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        assert_eq!(app.items.len(), 1);

        app.select(&conn).unwrap();
        assert!(matches!(app.nav_state, NavState::PageList { .. }));
        assert_eq!(app.items.len(), 1);
    }

    #[test]
    fn test_app_go_back() {
        let conn = setup_test_db();
        let space = repo::create_space(&conn, "s", "S", "").unwrap();
        repo::create_page(
            &conn, &space.id, None, "Page1", crate::models::PageType::Reference,
            "content", None, &[], "u", "a",
        ).unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        app.select(&conn).unwrap(); // Drill into space
        assert!(matches!(app.nav_state, NavState::PageList { .. }));

        app.go_back(&conn).unwrap();
        assert!(matches!(app.nav_state, NavState::SpaceList));
    }

    #[test]
    fn test_app_search() {
        let conn = setup_test_db();
        let space = repo::create_space(&conn, "s", "S", "").unwrap();
        repo::create_page(
            &conn, &space.id, None, "Rust Page", crate::models::PageType::Decision,
            "Rust is great", None, &[], "u", "a",
        ).unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();

        app.enter_search();
        assert_eq!(app.mode, Mode::Search);
        app.search_input = "Rust".to_string();
        app.submit_search(&conn).unwrap();

        assert!(matches!(app.nav_state, NavState::SearchResults { .. }));
        assert_eq!(app.items.len(), 1);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_app_nav_state_titles() {
        let app = App::new();
        assert_eq!(app.left_pane_title(), "Spaces");
    }

    #[test]
    fn test_prepare_edit_empty_list() {
        let conn = setup_test_db();
        let app = App::new();
        let result = app.prepare_edit(&conn).expect("prepare_edit should not error");
        assert!(result.is_none());
    }

    #[test]
    fn test_prepare_edit_space_returns_none() {
        let conn = setup_test_db();
        repo::create_space(&conn, "s", "S", "desc").unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        assert_eq!(app.items.len(), 1);

        let result = app.prepare_edit(&conn).expect("prepare_edit should not error");
        assert!(result.is_none());
    }

    #[test]
    fn test_prepare_edit_page_returns_edit_info() {
        let conn = setup_test_db();
        let space = repo::create_space(&conn, "s", "S", "").unwrap();
        let page = repo::create_page(
            &conn, &space.id, None, "My Page", crate::models::PageType::Reference,
            "Hello world", None, &[], "u", "a",
        ).unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        app.select(&conn).unwrap(); // Drill into space
        assert_eq!(app.items.len(), 1);

        let result = app.prepare_edit(&conn).expect("prepare_edit should not error");
        assert!(result.is_some());
        let (page_id, tmp_path, version) = result.unwrap();
        assert_eq!(page_id, page.id);
        assert_eq!(version, 1);
        // Verify temp file was written with the page content
        let content = std::fs::read_to_string(&tmp_path).expect("read temp file");
        assert_eq!(content, "Hello world");
        // Clean up
        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_prepare_edit_labels_empty_list() {
        let conn = setup_test_db();
        let app = App::new();
        let result = app.prepare_edit_labels(&conn).expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn test_prepare_edit_labels_space_returns_none() {
        let conn = setup_test_db();
        repo::create_space(&conn, "s", "S", "desc").unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        assert_eq!(app.items.len(), 1);

        let result = app.prepare_edit_labels(&conn).expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn test_prepare_edit_labels_page_returns_edit_info() {
        let conn = setup_test_db();
        let space = repo::create_space(&conn, "s", "S", "").unwrap();
        let page = repo::create_page(
            &conn, &space.id, None, "My Page", crate::models::PageType::Reference,
            "content", None, &["rust".to_string(), "testing".to_string()], "u", "a",
        ).unwrap();

        let mut app = App::new();
        app.load_initial(&conn).unwrap();
        app.select(&conn).unwrap(); // Drill into space
        assert_eq!(app.items.len(), 1);

        let result = app.prepare_edit_labels(&conn).expect("should not error");
        assert!(result.is_some());
        let (page_id, tmp_path, _version) = result.unwrap();
        assert_eq!(page_id, page.id);
        let content = std::fs::read_to_string(&tmp_path).expect("read temp file");
        assert!(content.contains("rust"));
        assert!(content.contains("testing"));
        assert!(content.contains("# Labels for: My Page"));
        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_list_item_display() {
        let space = Space {
            id: "id".into(),
            slug: "slug".into(),
            name: "Name".into(),
            description: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let item = ListItem::Space(space);
        assert_eq!(item.display_text(), "Name (slug)");
    }
}
