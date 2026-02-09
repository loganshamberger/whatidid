//! Full-text search and structured filtering for knowledge base pages.
//!
//! This module provides the core search functionality combining FTS5 full-text
//! search with structured metadata filters (space, page type, labels, agent).
//! When a text query is provided, it uses SQLite's FTS5 index for relevance
//! ranking and snippet extraction. Without a text query, it falls back to
//! efficient metadata-only filtering.

use crate::db::KbError;
use crate::models::{Page, PageType, SearchResult};
use rusqlite::Connection;

/// Parameters for searching pages in the knowledge base.
#[derive(Debug, Clone)]
pub struct SearchParams {
    /// Full-text search query. If None, only structured filters apply.
    pub query: Option<String>,
    /// Filter by space ID.
    pub space_id: Option<String>,
    /// Filter by page type.
    pub page_type: Option<PageType>,
    /// Filter by label.
    pub label: Option<String>,
    /// Filter by creating agent.
    pub created_by_agent: Option<String>,
    /// Filter to pages that have a specific section (by key name).
    pub section: Option<String>,
}

/// Generate an excerpt showing the query term in context within the content.
/// Returns up to ~100 characters centered around the first match.
fn make_excerpt(content: &str, query: &str) -> String {
    let lower_content = content.to_lowercase();
    // Strip FTS quoting from the query term.
    let clean_query = query.trim_matches('"').to_lowercase();

    if let Some(pos) = lower_content.find(&clean_query) {
        let start = pos.saturating_sub(40);
        let end = (pos + clean_query.len() + 40).min(content.len());
        let mut excerpt = String::new();
        if start > 0 {
            excerpt.push_str("...");
        }
        excerpt.push_str(&content[start..end]);
        if end < content.len() {
            excerpt.push_str("...");
        }
        excerpt
    } else {
        // Fallback: first 100 chars.
        if content.len() > 100 {
            format!("{}...", &content[..100])
        } else {
            content.to_string()
        }
    }
}

/// Search pages using full-text search and/or structured filters.
///
/// # Behavior
///
/// When `params.query` is Some:
/// - Uses FTS5 full-text search on page titles and content
/// - Returns results ordered by relevance (FTS5 rank)
/// - Includes excerpt snippets showing matched text in context
///
/// When `params.query` is None:
/// - Performs metadata-only filtering (no FTS5 join)
/// - Returns results in database order
/// - Excerpt field is empty string
///
/// Additional filters (space_id, page_type, label, created_by_agent) are
/// applied as WHERE clauses in both cases.
///
/// # Arguments
///
/// * `conn` - SQLite connection to the knowledge base
/// * `params` - Search parameters specifying query and filters
///
/// # Returns
///
/// A vector of SearchResult, each containing a Page and relevance excerpt.
/// Returns an empty vector if no pages match (not an error).
///
/// # Errors
///
/// Returns KbError::Db if the database query fails.
///
/// # Examples
///
/// ```ignore
/// // Full-text search
/// let results = search_pages(&conn, &SearchParams {
///     query: Some("rust concurrency".to_string()),
///     space_id: None,
///     page_type: None,
///     label: None,
///     created_by_agent: None,
/// })?;
///
/// // Metadata-only filter
/// let results = search_pages(&conn, &SearchParams {
///     query: None,
///     space_id: Some("project-x".to_string()),
///     page_type: Some(PageType::Decision),
///     label: Some("important".to_string()),
///     created_by_agent: Some("claude-code".to_string()),
/// })?;
/// ```
pub fn search_pages(conn: &Connection, params: &SearchParams) -> Result<Vec<SearchResult>, KbError> {
    let has_fts_query = params.query.is_some();

    // FTS queries use a subquery so that snippet() runs in a clean FTS context
    // (it breaks if combined with GROUP BY or complex joins). Labels are joined
    // outside the subquery.
    let mut query = String::new();

    if has_fts_query {
        // FTS5's snippet() doesn't work reliably with external content tables,
        // so we skip it and generate excerpts in Rust instead. FTS5 still
        // handles matching and ranking via the `rank` column.
        query.push_str(
            "SELECT p.id, p.space_id, p.parent_id, p.title, p.page_type, \
             p.content, p.created_by_user, p.created_by_agent, p.created_at, \
             p.updated_at, p.version, p.sections, '' as excerpt, \
             GROUP_CONCAT(DISTINCT all_labels.label) as label_list \
             FROM pages_fts \
             JOIN pages p ON p.rowid = pages_fts.rowid"
        );

        if params.label.is_some() {
            query.push_str(" LEFT JOIN labels filter_labels ON p.id = filter_labels.page_id");
        }

        query.push_str(" LEFT JOIN labels all_labels ON p.id = all_labels.page_id");

        // WHERE clause — always has MATCH.
        query.push_str(" WHERE pages_fts MATCH :query");
        if params.space_id.is_some() {
            query.push_str(" AND p.space_id = :space_id");
        }
        if params.page_type.is_some() {
            query.push_str(" AND p.page_type = :page_type");
        }
        if params.label.is_some() {
            query.push_str(" AND filter_labels.label = :label");
        }
        if params.created_by_agent.is_some() {
            query.push_str(" AND p.created_by_agent = :created_by_agent");
        }
        if params.section.is_some() {
            query.push_str(" AND json_extract(p.sections, :section_path) IS NOT NULL");
        }

        query.push_str(" GROUP BY p.id ORDER BY rank");
    } else {
        // Non-FTS: simple query from pages table.
        query.push_str(
            "SELECT p.id, p.space_id, p.parent_id, p.title, p.page_type, \
             p.content, p.created_by_user, p.created_by_agent, p.created_at, \
             p.updated_at, p.version, p.sections, '' as excerpt, \
             GROUP_CONCAT(DISTINCT all_labels.label) as label_list \
             FROM pages p"
        );

        if params.label.is_some() {
            query.push_str(" LEFT JOIN labels filter_labels ON p.id = filter_labels.page_id");
        }

        query.push_str(" LEFT JOIN labels all_labels ON p.id = all_labels.page_id");

        let mut where_clauses = Vec::new();
        if params.space_id.is_some() {
            where_clauses.push("p.space_id = :space_id".to_string());
        }
        if params.page_type.is_some() {
            where_clauses.push("p.page_type = :page_type".to_string());
        }
        if params.label.is_some() {
            where_clauses.push("filter_labels.label = :label".to_string());
        }
        if params.created_by_agent.is_some() {
            where_clauses.push("p.created_by_agent = :created_by_agent".to_string());
        }
        if params.section.is_some() {
            where_clauses.push("json_extract(p.sections, :section_path) IS NOT NULL".to_string());
        }

        if !where_clauses.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&where_clauses.join(" AND "));
        }

        query.push_str(" GROUP BY p.id");
    }

    // Prepare and bind only the parameters actually used in the query.
    // rusqlite rejects named parameters that don't appear in the SQL.
    let mut stmt = conn.prepare(&query)?;

    let mut bound_params: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
    if let Some(ref q) = params.query {
        // Quote the search term to prevent FTS5 syntax issues (e.g. hyphens
        // being interpreted as NOT operators or column filters).
        let quoted = format!("\"{}\"", q.replace('"', "\"\""));
        bound_params.push((":query", Box::new(quoted)));
    }
    if let Some(ref sid) = params.space_id {
        bound_params.push((":space_id", Box::new(sid.clone())));
    }
    if let Some(ref pt) = params.page_type {
        bound_params.push((":page_type", Box::new(pt.as_str().to_string())));
    }
    if let Some(ref lbl) = params.label {
        bound_params.push((":label", Box::new(lbl.clone())));
    }
    if let Some(ref agent) = params.created_by_agent {
        bound_params.push((":created_by_agent", Box::new(agent.clone())));
    }
    if let Some(ref section) = params.section {
        let path = format!("$.{}", section);
        bound_params.push((":section_path", Box::new(path)));
    }

    let param_slice: Vec<(&str, &dyn rusqlite::types::ToSql)> = bound_params
        .iter()
        .map(|(name, val)| (*name, val.as_ref() as &dyn rusqlite::types::ToSql))
        .collect();

    // Column indices (same for both FTS and non-FTS paths):
    // 0:id  1:space_id  2:parent_id  3:title  4:page_type  5:content
    // 6:created_by_user  7:created_by_agent  8:created_at  9:updated_at
    // 10:version  11:sections  12:excerpt  13:label_list
    let rows = stmt.query_map(param_slice.as_slice(), |row| {
            let page_type_str: String = row.get(4)?;
            let page_type = PageType::from_str(&page_type_str)
                .ok_or_else(|| rusqlite::Error::InvalidParameterName(
                    format!("Invalid page_type: {}", page_type_str)
                ))?;

            let sections_str: Option<String> = row.get(11)?;
            let sections: Option<serde_json::Value> = sections_str
                .and_then(|s| serde_json::from_str(&s).ok());

            let label_list_opt: Option<String> = row.get(13)?;
            let labels: Vec<String> = label_list_opt
                .map(|s| s.split(',').map(|l| l.to_string()).collect())
                .unwrap_or_default();

            Ok(SearchResult {
                page: Page {
                    id: row.get(0)?,
                    space_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    title: row.get(3)?,
                    page_type,
                    content: row.get(5)?,
                    sections,
                    created_by_user: row.get(6)?,
                    created_by_agent: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    version: row.get(10)?,
                    labels,
                },
                excerpt: row.get(12)?,
            })
        },
    )?;

    let mut results = Vec::new();
    for row_result in rows {
        let mut result = row_result?;
        // Generate excerpt in Rust for FTS queries (since snippet() doesn't
        // work with external content FTS5 tables).
        if has_fts_query && result.excerpt.is_empty() {
            if let Some(ref q) = params.query {
                // If searching within a specific section, excerpt from that section
                if let Some(ref section_key) = params.section {
                    if let Some(ref sections) = result.page.sections {
                        if let Some(section_text) = sections.get(section_key).and_then(|v| v.as_str()) {
                            result.excerpt = make_excerpt(section_text, q);
                        } else {
                            result.excerpt = make_excerpt(&result.page.content, q);
                        }
                    } else {
                        result.excerpt = make_excerpt(&result.page.content, q);
                    }
                } else {
                    result.excerpt = make_excerpt(&result.page.content, q);
                }
            }
        }
        results.push(result);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Create an in-memory database with the full schema for testing.
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("Failed to create in-memory DB");

        // Execute the migration SQL
        let migration_sql = include_str!("../migrations/001_initial.sql");
        conn.execute_batch(migration_sql)
            .expect("Failed to execute migration 001");

        let migration2_sql = include_str!("../migrations/002_sections.sql");
        conn.execute_batch(migration2_sql)
            .expect("Failed to execute migration 002");

        let migration3_sql = include_str!("../migrations/003_timestamps.sql");
        conn.execute_batch(migration3_sql)
            .expect("Failed to execute migration 003");

        conn
    }

    /// Insert test data: space, pages with various properties
    fn insert_test_data(conn: &Connection) {
        // Create a test space
        conn.execute(
            "INSERT INTO spaces (id, slug, name, description, created_at)
             VALUES ('space-1', 'test-space', 'Test Space', 'A test space', datetime('now'))",
            [],
        ).expect("Failed to insert space");

        // Page 1: Decision about Rust with label "important"
        conn.execute(
            "INSERT INTO pages (id, space_id, parent_id, title, page_type, content,
                                created_by_user, created_by_agent, created_at, updated_at, version)
             VALUES ('page-1', 'space-1', NULL, 'Use Rust for CLI', 'decision',
                     'We decided to use Rust because of its memory safety and performance guarantees. Rust concurrency model is excellent.',
                     'logan', 'claude-code', datetime('now'), datetime('now'), 1)",
            [],
        ).expect("Failed to insert page 1");

        conn.execute(
            "INSERT INTO labels (page_id, label) VALUES ('page-1', 'important')",
            [],
        ).expect("Failed to insert label for page 1");

        // Page 2: Architecture doc about SQLite
        conn.execute(
            "INSERT INTO pages (id, space_id, parent_id, title, page_type, content,
                                created_by_user, created_by_agent, created_at, updated_at, version)
             VALUES ('page-2', 'space-1', NULL, 'Database Architecture', 'architecture',
                     'We use SQLite for local storage. FTS5 provides full-text search capabilities.',
                     'logan', 'claude-code', datetime('now'), datetime('now'), 1)",
            [],
        ).expect("Failed to insert page 2");

        conn.execute(
            "INSERT INTO labels (page_id, label) VALUES ('page-2', 'database')",
            [],
        ).expect("Failed to insert label for page 2");

        // Page 3: Reference doc about testing, different agent
        conn.execute(
            "INSERT INTO pages (id, space_id, parent_id, title, page_type, content,
                                created_by_user, created_by_agent, created_at, updated_at, version)
             VALUES ('page-3', 'space-1', NULL, 'Testing Guidelines', 'reference',
                     'All code must have unit tests. Use the built-in Rust testing framework.',
                     'logan', 'copilot', datetime('now'), datetime('now'), 1)",
            [],
        ).expect("Failed to insert page 3");

        conn.execute(
            "INSERT INTO labels (page_id, label) VALUES ('page-3', 'testing')",
            [],
        ).expect("Failed to insert label for page 3");

        // Page 4: No labels, contains "Rust" for FTS testing
        conn.execute(
            "INSERT INTO pages (id, space_id, parent_id, title, page_type, content,
                                created_by_user, created_by_agent, created_at, updated_at, version)
             VALUES ('page-4', 'space-1', NULL, 'Rust Best Practices', 'runbook',
                     'Follow idiomatic Rust patterns. Avoid unsafe code unless necessary.',
                     'logan', 'claude-code', datetime('now'), datetime('now'), 1)",
            [],
        ).expect("Failed to insert page 4");

        // Create a second space with one page
        conn.execute(
            "INSERT INTO spaces (id, slug, name, description, created_at)
             VALUES ('space-2', 'other-space', 'Other Space', 'Another space', datetime('now'))",
            [],
        ).expect("Failed to insert space 2");

        conn.execute(
            "INSERT INTO pages (id, space_id, parent_id, title, page_type, content,
                                created_by_user, created_by_agent, created_at, updated_at, version)
             VALUES ('page-5', 'space-2', NULL, 'Different Space Page', 'reference',
                     'This page is in a different space and mentions Rust.',
                     'logan', 'claude-code', datetime('now'), datetime('now'), 1)",
            [],
        ).expect("Failed to insert page 5");
    }

    #[test]
    fn test_fts_search_finds_matching_pages() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        let results = search_pages(&conn, &SearchParams {
            query: Some("Rust".to_string()),
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        // Should find pages 1, 3, 4, and 5 which contain "Rust"
        // (page 3 mentions "built-in Rust testing framework")
        assert_eq!(results.len(), 4);

        let ids: Vec<&str> = results.iter().map(|r| r.page.id.as_str()).collect();
        assert!(ids.contains(&"page-1"));
        assert!(ids.contains(&"page-3"));
        assert!(ids.contains(&"page-4"));
        assert!(ids.contains(&"page-5"));
    }

    #[test]
    fn test_fts_search_returns_snippets() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        let results = search_pages(&conn, &SearchParams {
            query: Some("Rust".to_string()),
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        // Every result should have a non-empty excerpt containing the search term
        for result in &results {
            assert!(!result.excerpt.is_empty(), "Excerpt should not be empty for FTS search");
            assert!(
                result.excerpt.to_lowercase().contains("rust"),
                "Excerpt should contain the search term: {}",
                result.excerpt
            );
        }
    }

    #[test]
    fn test_metadata_filter_without_fts() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Filter by page_type only, no FTS query
        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: Some(PageType::Decision),
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-1");
        assert_eq!(results[0].excerpt, "", "Excerpt should be empty for non-FTS search");
    }

    #[test]
    fn test_filter_by_space_id() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: Some("space-2".to_string()),
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-5");
    }

    #[test]
    fn test_filter_by_label() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: None,
            label: Some("important".to_string()),
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-1");
        assert!(results[0].page.labels.contains(&"important".to_string()));
    }

    #[test]
    fn test_filter_by_agent() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: Some("copilot".to_string()),
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-3");
    }

    #[test]
    fn test_combined_fts_and_filters() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Search for "Rust" but only in space-1
        let results = search_pages(&conn, &SearchParams {
            query: Some("Rust".to_string()),
            space_id: Some("space-1".to_string()),
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        // Should find pages 1, 3, and 4 (not page-5 which is in space-2)
        // (page 3 mentions "built-in Rust testing framework")
        assert_eq!(results.len(), 3);
        let ids: Vec<&str> = results.iter().map(|r| r.page.id.as_str()).collect();
        assert!(ids.contains(&"page-1"));
        assert!(ids.contains(&"page-3"));
        assert!(ids.contains(&"page-4"));
    }

    #[test]
    fn test_combined_multiple_filters() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Filter by space, page_type, and agent
        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: Some("space-1".to_string()),
            page_type: Some(PageType::Decision),
            label: None,
            created_by_agent: Some("claude-code".to_string()),
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-1");
    }

    #[test]
    fn test_empty_results_not_an_error() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Search for something that doesn't exist
        let results = search_pages(&conn, &SearchParams {
            query: Some("nonexistent-term-xyz".to_string()),
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed even with no results");

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_labels_populated_correctly() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Get page 1 which has one label
        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: Some(PageType::Decision),
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.labels, vec!["important"]);

        // Get page 4 which has no labels
        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: Some(PageType::Runbook),
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.labels.len(), 0);
    }

    #[test]
    fn test_page_with_multiple_labels() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Add multiple labels to a page
        conn.execute(
            "INSERT INTO labels (page_id, label) VALUES ('page-1', 'reviewed')",
            [],
        ).expect("Failed to insert additional label");

        conn.execute(
            "INSERT INTO labels (page_id, label) VALUES ('page-1', 'archived')",
            [],
        ).expect("Failed to insert additional label");

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: Some(PageType::Decision),
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.labels.len(), 3);
        assert!(results[0].page.labels.contains(&"important".to_string()));
        assert!(results[0].page.labels.contains(&"reviewed".to_string()));
        assert!(results[0].page.labels.contains(&"archived".to_string()));
    }

    #[test]
    fn test_fts_search_in_title() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Search for a word that appears in the title
        let results = search_pages(&conn, &SearchParams {
            query: Some("Database".to_string()),
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-2");
        assert!(results[0].page.title.contains("Database"));
    }

    #[test]
    fn test_no_filters_returns_all_pages() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("Search should succeed");

        // Should return all 5 pages
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_filter_by_section() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Add sections to page-1
        conn.execute(
            "UPDATE pages SET sections = ? WHERE id = 'page-1'",
            [r#"{"context":"why we chose rust","decision":"use rust"}"#],
        ).expect("update sections");

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: Some("context".to_string()),
        }).expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-1");
    }

    #[test]
    fn test_fts_with_section_filter() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        // Add sections to page-1
        conn.execute(
            "UPDATE pages SET sections = ? WHERE id = 'page-1'",
            [r#"{"context":"why we chose rust","decision":"use rust"}"#],
        ).expect("update sections");

        let results = search_pages(&conn, &SearchParams {
            query: Some("Rust".to_string()),
            space_id: None,
            page_type: None,
            label: None,
            created_by_agent: None,
            section: Some("context".to_string()),
        }).expect("search");

        // Only page-1 has sections with 'context' key AND matches 'Rust'
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page.id, "page-1");
    }

    #[test]
    fn test_search_results_include_sections() {
        let conn = setup_test_db();
        insert_test_data(&conn);

        conn.execute(
            "UPDATE pages SET sections = ? WHERE id = 'page-1'",
            [r#"{"context":"test context","decision":"test decision"}"#],
        ).expect("update sections");

        let results = search_pages(&conn, &SearchParams {
            query: None,
            space_id: None,
            page_type: Some(PageType::Decision),
            label: None,
            created_by_agent: None,
            section: None,
        }).expect("search");

        assert_eq!(results.len(), 1);
        assert!(results[0].page.sections.is_some());
    }
}
