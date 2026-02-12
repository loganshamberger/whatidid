//! Repository layer: all SQL queries for CRUD operations.
//!
//! This module provides plain functions that execute SQL statements using
//! `rusqlite::Connection`. Each function takes a database connection as its
//! first parameter and returns a `Result<T, KbError>`.
//!
//! No traits, no generics, no repository pattern — just simple functions that
//! map between Rust structs and SQLite tables.

use crate::db::KbError;
use crate::models::{sections_to_content, Link, LinkRelation, Page, PageType, Space};
use rusqlite::Connection;

/// Filters for listing pages with structured queries.
pub struct PageFilters {
    pub space_id: Option<String>,
    pub page_type: Option<PageType>,
    pub label: Option<String>,
    pub created_by_user: Option<String>,
    pub created_by_agent: Option<String>,
}

/// Map a rusqlite Row to a Page struct.
/// Expects columns in order: id, space_id, parent_id, title, page_type, content,
/// created_by_user, created_by_agent, created_at, updated_at, version, sections
fn row_to_page(row: &rusqlite::Row) -> Result<Page, rusqlite::Error> {
    let page_type_str: String = row.get(4)?;
    let page_type = PageType::from_str(&page_type_str)
        .ok_or_else(|| rusqlite::Error::InvalidColumnType(4, "page_type".to_string(), rusqlite::types::Type::Text))?;
    let sections_str: Option<String> = row.get(11)?;
    let sections: Option<serde_json::Value> = sections_str
        .and_then(|s| serde_json::from_str(&s).ok());
    Ok(Page {
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
        labels: vec![],
    })
}

// =============================================================================
// Spaces
// =============================================================================

/// Creates a new space with a generated UUID and current timestamp.
///
/// # Arguments
/// * `conn` - Database connection
/// * `slug` - URL-friendly identifier for the space (must be unique)
/// * `name` - Display name for the space
/// * `description` - Optional description of the space's purpose
///
/// # Returns
/// The newly created space
///
/// # Errors
/// Returns `KbError::Db` if the slug is not unique or the insert fails.
pub fn create_space(
    conn: &Connection,
    slug: &str,
    name: &str,
    description: &str,
) -> Result<Space, KbError> {
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO spaces (id, slug, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, slug, name, description, created_at, created_at],
    )
    .map_err(KbError::Db)?;

    Ok(Space {
        id,
        slug: slug.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        created_at: created_at.clone(),
        updated_at: created_at,
    })
}

/// Retrieves a space by its slug.
///
/// # Arguments
/// * `conn` - Database connection
/// * `slug` - The space's unique slug identifier
///
/// # Returns
/// The matching space
///
/// # Errors
/// Returns `KbError::NotFound` if no space with the given slug exists.
pub fn get_space_by_slug(conn: &Connection, slug: &str) -> Result<Space, KbError> {
    let mut stmt = conn
        .prepare("SELECT id, slug, name, description, created_at, updated_at FROM spaces WHERE slug = ?1")
        .map_err(KbError::Db)?;

    let space = stmt
        .query_row([slug], |row| {
            Ok(Space {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                description: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                KbError::NotFound(format!("Space with slug '{}' not found", slug))
            }
            _ => KbError::Db(e),
        })?;

    Ok(space)
}

/// Lists all spaces in the knowledge base.
///
/// # Arguments
/// * `conn` - Database connection
///
/// # Returns
/// A vector of all spaces, ordered by creation date (newest first)
pub fn list_spaces(conn: &Connection) -> Result<Vec<Space>, KbError> {
    let mut stmt = conn
        .prepare("SELECT id, slug, name, description, created_at, updated_at FROM spaces ORDER BY created_at DESC")
        .map_err(KbError::Db)?;

    let spaces = stmt
        .query_map([], |row| {
            Ok(Space {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                description: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(KbError::Db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KbError::Db)?;

    Ok(spaces)
}

/// Deletes a space by its slug.
///
/// # Arguments
/// * `conn` - Database connection
/// * `slug` - The space's unique slug identifier
///
/// # Returns
/// Unit on success
///
/// # Errors
/// Returns `KbError::Db` if the space has pages (foreign key constraint violation)
/// or if the space does not exist.
pub fn delete_space(conn: &Connection, slug: &str) -> Result<(), KbError> {
    // First verify the space exists by getting its ID
    let space = get_space_by_slug(conn, slug)?;

    conn.execute("DELETE FROM spaces WHERE id = ?1", [&space.id])
        .map_err(KbError::Db)?;

    Ok(())
}

// =============================================================================
// Pages
// =============================================================================

/// Creates a new page with generated UUID, current timestamps, and version 1.
///
/// # Arguments
/// * `conn` - Database connection
/// * `space_id` - ID of the space this page belongs to
/// * `parent_id` - Optional parent page ID for hierarchical organization
/// * `title` - Page title
/// * `page_type` - Type of page (decision, architecture, etc.)
/// * `content` - Page content in markdown format
/// * `sections` - Optional structured sections JSON
/// * `labels` - Tags to attach to this page
/// * `user` - User who created this page
/// * `agent` - Agent tool that created this page
///
/// # Returns
/// The newly created page with labels populated
pub fn create_page(
    conn: &Connection,
    space_id: &str,
    parent_id: Option<&str>,
    title: &str,
    page_type: PageType,
    content: &str,
    sections: Option<&serde_json::Value>,
    labels: &[String],
    user: &str,
    agent: &str,
) -> Result<Page, KbError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Determine effective content: if sections provided and content empty, derive from sections
    let effective_content = if let Some(secs) = sections {
        if content.is_empty() {
            sections_to_content(secs, page_type)
        } else {
            content.to_string()
        }
    } else {
        content.to_string()
    };

    // Validate sections against schema (warn, don't reject)
    if let Some(secs) = sections {
        if let Some(schema) = page_type.section_schema() {
            if let Some(obj) = secs.as_object() {
                // Warn about unknown section names
                for key in obj.keys() {
                    if !schema.iter().any(|d| d.key == key) {
                        eprintln!("Warning: unknown section '{}' for page type '{}'", key, page_type);
                    }
                }
                // Warn about missing required sections
                for def in &schema {
                    if def.required && !obj.contains_key(def.key) {
                        eprintln!("Warning: missing required section '{}' for page type '{}'", def.key, page_type);
                    }
                }
            }
        }
    }

    let sections_json: Option<String> = sections.map(|s| serde_json::to_string(s).unwrap());

    // Wrap page INSERT + label INSERTs in a transaction
    let tx = conn.unchecked_transaction()?;

    tx.execute(
        "INSERT INTO pages (id, space_id, parent_id, title, page_type, content, sections, created_by_user, created_by_agent, created_at, updated_at, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)",
        rusqlite::params![
            id,
            space_id,
            parent_id,
            title,
            page_type.as_str(),
            effective_content,
            sections_json,
            user,
            agent,
            now,
            now,
        ],
    )
    .map_err(KbError::Db)?;

    // Insert labels within the same transaction
    if !labels.is_empty() {
        for label in labels {
            tx.execute(
                "INSERT INTO labels (page_id, label) VALUES (?1, ?2)",
                rusqlite::params![id, label],
            )
            .map_err(KbError::Db)?;
        }
    }

    tx.commit()?;

    Ok(Page {
        id,
        space_id: space_id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        title: title.to_string(),
        page_type,
        content: effective_content,
        sections: sections.cloned(),
        created_by_user: user.to_string(),
        created_by_agent: agent.to_string(),
        created_at: now.clone(),
        updated_at: now,
        version: 1,
        labels: labels.to_vec(),
    })
}

/// Retrieves a page by its ID, with labels populated.
///
/// # Arguments
/// * `conn` - Database connection
/// * `id` - The page's unique ID
///
/// # Returns
/// The matching page with its labels
///
/// # Errors
/// Returns `KbError::NotFound` if no page with the given ID exists.
pub fn get_page(conn: &Connection, id: &str) -> Result<Page, KbError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, space_id, parent_id, title, page_type, content,
                    created_by_user, created_by_agent, created_at, updated_at, version, sections
             FROM pages WHERE id = ?1",
        )
        .map_err(KbError::Db)?;

    let page = stmt
        .query_row([id], |row| row_to_page(row))
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                KbError::NotFound(format!("Page with ID '{}' not found", id))
            }
            _ => KbError::Db(e),
        })?;

    // Fetch labels for this page
    let labels = get_labels(conn, id)?;

    Ok(Page { labels, ..page })
}

/// Updates a page's title and/or content with optimistic concurrency control.
///
/// # Arguments
/// * `conn` - Database connection
/// * `id` - The page's unique ID
/// * `title` - New title (if Some)
/// * `content` - New content (if Some)
/// * `sections` - New sections (if Some)
/// * `expected_version` - Expected current version for optimistic locking (if Some)
///
/// # Returns
/// The updated page with incremented version and updated timestamp
///
/// # Errors
/// Returns `KbError::VersionConflict` if the expected version doesn't match.
/// Returns `KbError::NotFound` if the page doesn't exist.
pub fn update_page(
    conn: &Connection,
    id: &str,
    title: Option<&str>,
    content: Option<&str>,
    sections: Option<&serde_json::Value>,
    expected_version: Option<i64>,
) -> Result<Page, KbError> {
    let now = chrono::Utc::now().to_rfc3339();

    // Determine effective content and sections JSON
    let (effective_content, sections_json) = if let Some(secs) = sections {
        // Get existing page to know the type for content generation
        let existing = get_page(conn, id)?;
        let merged_content = sections_to_content(secs, existing.page_type);
        let json = serde_json::to_string(secs).unwrap();
        (Some(merged_content), Some(json))
    } else {
        (content.map(|s| s.to_string()), None)
    };

    // Perform atomic update with optional version check
    let rows_affected = if let Some(expected) = expected_version {
        // With version check: UPDATE ... WHERE id = ? AND version = ?
        conn.execute(
            "UPDATE pages
             SET title = COALESCE(?1, title),
                 content = COALESCE(?2, content),
                 sections = COALESCE(?3, sections),
                 updated_at = ?4,
                 version = version + 1
             WHERE id = ?5 AND version = ?6",
            rusqlite::params![title, effective_content, sections_json, now, id, expected],
        )
        .map_err(KbError::Db)?
    } else {
        // Without version check: UPDATE ... WHERE id = ?
        conn.execute(
            "UPDATE pages
             SET title = COALESCE(?1, title),
                 content = COALESCE(?2, content),
                 sections = COALESCE(?3, sections),
                 updated_at = ?4,
                 version = version + 1
             WHERE id = ?5",
            rusqlite::params![title, effective_content, sections_json, now, id],
        )
        .map_err(KbError::Db)?
    };

    // If no rows were affected, determine why
    if rows_affected == 0 {
        // Try to get the page to determine if it exists
        match get_page(conn, id) {
            Ok(page) => {
                // Page exists, so the version must have been wrong
                if let Some(expected) = expected_version {
                    return Err(KbError::VersionConflict {
                        expected,
                        actual: page.version,
                    });
                } else {
                    // This shouldn't happen, but return NotFound as fallback
                    return Err(KbError::NotFound(format!("Page with ID '{}' not found", id)));
                }
            }
            Err(KbError::NotFound(_)) => {
                // Page doesn't exist
                return Err(KbError::NotFound(format!("Page with ID '{}' not found", id)));
            }
            Err(e) => return Err(e),
        }
    }

    // Return the updated page
    get_page(conn, id)
}

/// Appends content to an existing page's content.
///
/// # Arguments
/// * `conn` - Database connection
/// * `id` - The page's unique ID
/// * `content_to_append` - Text to append to the current content
///
/// # Returns
/// The updated page with incremented version and updated timestamp
///
/// # Errors
/// Returns `KbError::NotFound` if the page doesn't exist.
pub fn append_to_page(conn: &Connection, id: &str, content_to_append: &str) -> Result<Page, KbError> {
    let now = chrono::Utc::now().to_rfc3339();

    // Perform atomic append using SQL concatenation
    // Use CASE to handle empty content (no leading newline)
    let rows_affected = conn
        .execute(
            "UPDATE pages
             SET content = CASE
                 WHEN content = '' THEN ?1
                 ELSE content || char(10) || ?1
             END,
             updated_at = ?2,
             version = version + 1
             WHERE id = ?3",
            rusqlite::params![content_to_append, now, id],
        )
        .map_err(KbError::Db)?;

    if rows_affected == 0 {
        return Err(KbError::NotFound(format!("Page with ID '{}' not found", id)));
    }

    get_page(conn, id)
}

/// Lists pages matching the given filters.
///
/// # Arguments
/// * `conn` - Database connection
/// * `filters` - Filters to apply (all are optional, unset filters are ignored)
///
/// # Returns
/// A vector of pages matching all provided filters
pub fn list_pages(conn: &Connection, filters: &PageFilters) -> Result<Vec<Page>, KbError> {
    let mut sql = String::from(
        "SELECT DISTINCT p.id, p.space_id, p.parent_id, p.title, p.page_type, p.content,
                p.created_by_user, p.created_by_agent, p.created_at, p.updated_at, p.version, p.sections
         FROM pages p",
    );

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    // Join with labels if filtering by label
    if filters.label.is_some() {
        sql.push_str(" INNER JOIN labels l ON p.id = l.page_id");
    }

    // Build WHERE conditions
    if let Some(ref space_id) = filters.space_id {
        conditions.push("p.space_id = ?".to_string());
        params.push(Box::new(space_id.clone()));
    }

    if let Some(ref page_type) = filters.page_type {
        conditions.push("p.page_type = ?".to_string());
        params.push(Box::new(page_type.as_str().to_string()));
    }

    if let Some(ref label) = filters.label {
        conditions.push("l.label = ?".to_string());
        params.push(Box::new(label.clone()));
    }

    if let Some(ref user) = filters.created_by_user {
        conditions.push("p.created_by_user = ?".to_string());
        params.push(Box::new(user.clone()));
    }

    if let Some(ref agent) = filters.created_by_agent {
        conditions.push("p.created_by_agent = ?".to_string());
        params.push(Box::new(agent.clone()));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY p.created_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(KbError::Db)?;

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let pages = stmt
        .query_map(&param_refs[..], |row| row_to_page(row))
        .map_err(KbError::Db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KbError::Db)?;

    // Populate labels for each page
    let mut pages_with_labels = Vec::new();
    for page in pages {
        let labels = get_labels(conn, &page.id)?;
        pages_with_labels.push(Page { labels, ..page });
    }

    Ok(pages_with_labels)
}

/// Deletes a page and all associated labels and links (via CASCADE).
///
/// # Arguments
/// * `conn` - Database connection
/// * `id` - The page's unique ID
///
/// # Returns
/// Unit on success
///
/// # Errors
/// Returns `KbError::Db` if the deletion fails.
pub fn delete_page(conn: &Connection, id: &str) -> Result<(), KbError> {
    let rows_affected = conn
        .execute("DELETE FROM pages WHERE id = ?1", [id])
        .map_err(KbError::Db)?;

    if rows_affected == 0 {
        return Err(KbError::NotFound(format!("Page with ID '{}' not found", id)));
    }

    Ok(())
}

// =============================================================================
// Labels
// =============================================================================

/// Replaces all labels for a page with the given set.
///
/// # Arguments
/// * `conn` - Database connection
/// * `page_id` - The page's unique ID
/// * `labels` - New set of labels to attach to the page
///
/// # Returns
/// Unit on success
///
/// # Errors
/// Returns `KbError::Db` if the operation fails.
pub fn set_labels(conn: &Connection, page_id: &str, labels: &[String]) -> Result<(), KbError> {
    // Wrap DELETE + INSERTs in a transaction
    let tx = conn.unchecked_transaction()?;

    // Delete existing labels
    tx.execute("DELETE FROM labels WHERE page_id = ?1", [page_id])
        .map_err(KbError::Db)?;

    // Insert new labels
    for label in labels {
        tx.execute(
            "INSERT INTO labels (page_id, label) VALUES (?1, ?2)",
            rusqlite::params![page_id, label],
        )
        .map_err(KbError::Db)?;
    }

    tx.commit()?;

    Ok(())
}

/// Adds a single label to a page, ignoring duplicates.
///
/// Uses `INSERT OR IGNORE` for idempotency — if the label already exists on
/// the page (UNIQUE constraint on `(page_id, label)`), the operation is a no-op.
///
/// # Arguments
/// * `conn` - Database connection
/// * `page_id` - The page's unique ID
/// * `label` - The label string to add
///
/// # Errors
/// Returns `KbError::Db` if the operation fails (e.g., invalid page_id).
pub fn add_label(conn: &Connection, page_id: &str, label: &str) -> Result<(), KbError> {
    conn.execute(
        "INSERT OR IGNORE INTO labels (page_id, label) VALUES (?1, ?2)",
        rusqlite::params![page_id, label],
    )
    .map_err(KbError::Db)?;
    Ok(())
}

/// Retrieves all labels for a page.
///
/// # Arguments
/// * `conn` - Database connection
/// * `page_id` - The page's unique ID
///
/// # Returns
/// A vector of label strings attached to the page
pub fn get_labels(conn: &Connection, page_id: &str) -> Result<Vec<String>, KbError> {
    let mut stmt = conn
        .prepare("SELECT label FROM labels WHERE page_id = ?1 ORDER BY label")
        .map_err(KbError::Db)?;

    let labels = stmt
        .query_map([page_id], |row| row.get(0))
        .map_err(KbError::Db)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(KbError::Db)?;

    Ok(labels)
}

// =============================================================================
// Links
// =============================================================================

/// Creates a typed relationship between two pages.
///
/// # Arguments
/// * `conn` - Database connection
/// * `source_id` - ID of the source page
/// * `target_id` - ID of the target page
/// * `relation` - Type of relationship
///
/// # Returns
/// The newly created link
///
/// # Errors
/// Returns `KbError::Db` if the link already exists or references nonexistent pages.
pub fn create_link(
    conn: &Connection,
    source_id: &str,
    target_id: &str,
    relation: LinkRelation,
) -> Result<Link, KbError> {
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO links (source_id, target_id, relation, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![source_id, target_id, relation.as_str(), now, now],
    )
    .map_err(KbError::Db)?;

    Ok(Link {
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        relation,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Lists all links where the given page is either the source or target.
///
/// # Arguments
/// * `conn` - Database connection
/// * `page_id` - The page's unique ID
///
/// # Returns
/// A vector of links involving the page
pub fn list_links(conn: &Connection, page_id: &str) -> Result<Vec<Link>, KbError> {
    let mut stmt = conn
        .prepare(
            "SELECT source_id, target_id, relation, created_at, updated_at
             FROM links
             WHERE source_id = ?1 OR target_id = ?1",
        )
        .map_err(KbError::Db)?;

    let links = stmt
        .query_map([page_id], |row| {
            let relation_str: String = row.get(2)?;
            let relation = LinkRelation::from_str(&relation_str)
                .ok_or_else(|| rusqlite::Error::InvalidColumnType(2, "relation".to_string(), rusqlite::types::Type::Text))?;

            Ok(Link {
                source_id: row.get(0)?,
                target_id: row.get(1)?,
                relation,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .map_err(KbError::Db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KbError::Db)?;

    Ok(links)
}

/// Deletes a link between two pages.
///
/// # Arguments
/// * `conn` - Database connection
/// * `source_id` - ID of the source page
/// * `target_id` - ID of the target page
///
/// # Returns
/// Unit on success
///
/// # Errors
/// Returns `KbError::Db` if the deletion fails.
pub fn delete_link(conn: &Connection, source_id: &str, target_id: &str) -> Result<(), KbError> {
    let rows_affected = conn
        .execute(
            "DELETE FROM links WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![source_id, target_id],
        )
        .map_err(KbError::Db)?;

    if rows_affected == 0 {
        return Err(KbError::NotFound(format!(
            "Link from '{}' to '{}' not found",
            source_id, target_id
        )));
    }

    Ok(())
}

// =============================================================================
// TUI navigation helpers
// =============================================================================

/// Lists top-level pages (no parent) in a space, ordered by title.
pub fn list_top_level_pages(conn: &Connection, space_id: &str) -> Result<Vec<Page>, KbError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, space_id, parent_id, title, page_type, content,
                    created_by_user, created_by_agent, created_at, updated_at, version, sections
             FROM pages WHERE space_id = ?1 AND parent_id IS NULL
             ORDER BY title COLLATE NOCASE",
        )
        .map_err(KbError::Db)?;

    let pages = stmt
        .query_map([space_id], |row| row_to_page(row))
        .map_err(KbError::Db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KbError::Db)?;

    let mut pages_with_labels = Vec::new();
    for page in pages {
        let labels = get_labels(conn, &page.id)?;
        pages_with_labels.push(Page { labels, ..page });
    }
    Ok(pages_with_labels)
}

/// Lists child pages of a given parent page, ordered by title.
pub fn list_child_pages(conn: &Connection, parent_id: &str) -> Result<Vec<Page>, KbError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, space_id, parent_id, title, page_type, content,
                    created_by_user, created_by_agent, created_at, updated_at, version, sections
             FROM pages WHERE parent_id = ?1
             ORDER BY title COLLATE NOCASE",
        )
        .map_err(KbError::Db)?;

    let pages = stmt
        .query_map([parent_id], |row| row_to_page(row))
        .map_err(KbError::Db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KbError::Db)?;

    let mut pages_with_labels = Vec::new();
    for page in pages {
        let labels = get_labels(conn, &page.id)?;
        pages_with_labels.push(Page { labels, ..page });
    }
    Ok(pages_with_labels)
}

/// Returns true if a page has any child pages.
pub fn has_children(conn: &Connection, page_id: &str) -> Result<bool, KbError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE parent_id = ?1 LIMIT 1",
            [page_id],
            |row| row.get(0),
        )
        .map_err(KbError::Db)?;
    Ok(count > 0)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("Failed to create in-memory database");
        let migration_sql = include_str!("../migrations/001_initial.sql");
        conn.execute_batch(migration_sql)
            .expect("Failed to run migration 001");
        let migration2_sql = include_str!("../migrations/002_sections.sql");
        conn.execute_batch(migration2_sql)
            .expect("Failed to run migration 002");
        let migration3_sql = include_str!("../migrations/003_timestamps.sql");
        conn.execute_batch(migration3_sql)
            .expect("Failed to run migration 003");
        conn
    }

    #[test]
    fn test_create_and_get_space() {
        let conn = setup_test_db();

        let space = create_space(
            &conn,
            "test-space",
            "Test Space",
            "A test space for testing",
        )
        .expect("Failed to create space");

        assert_eq!(space.slug, "test-space");
        assert_eq!(space.name, "Test Space");
        assert_eq!(space.description, "A test space for testing");
        assert!(!space.id.is_empty());
        assert!(!space.created_at.is_empty());

        let retrieved = get_space_by_slug(&conn, "test-space").expect("Failed to get space");
        assert_eq!(retrieved.id, space.id);
        assert_eq!(retrieved.slug, space.slug);
        assert_eq!(retrieved.name, space.name);
    }

    #[test]
    fn test_get_space_not_found() {
        let conn = setup_test_db();

        let result = get_space_by_slug(&conn, "nonexistent");
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_list_spaces() {
        let conn = setup_test_db();

        create_space(&conn, "space1", "Space One", "First space").expect("Failed to create space1");
        create_space(&conn, "space2", "Space Two", "Second space").expect("Failed to create space2");

        let spaces = list_spaces(&conn).expect("Failed to list spaces");
        assert_eq!(spaces.len(), 2);
        // Should be ordered by created_at DESC, so space2 first
        assert_eq!(spaces[0].slug, "space2");
        assert_eq!(spaces[1].slug, "space1");
    }

    #[test]
    fn test_delete_space() {
        let conn = setup_test_db();

        create_space(&conn, "temp-space", "Temp", "Temporary").expect("Failed to create space");
        delete_space(&conn, "temp-space").expect("Failed to delete space");

        let result = get_space_by_slug(&conn, "temp-space");
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_delete_space_with_pages_fails() {
        let conn = setup_test_db();

        let space = create_space(&conn, "space-with-pages", "Space", "").expect("Failed to create space");
        create_page(
            &conn,
            &space.id,
            None,
            "Test Page",
            PageType::Reference,
            "Content",
            None,
            &[],
            "testuser",
            "testagent",
        )
        .expect("Failed to create page");

        let result = delete_space(&conn, "space-with-pages");
        assert!(matches!(result, Err(KbError::Db(_))));
    }

    #[test]
    fn test_create_and_get_page_with_labels() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let labels = vec!["rust".to_string(), "testing".to_string()];

        let page = create_page(
            &conn,
            &space.id,
            None,
            "Test Page",
            PageType::Decision,
            "# Decision\n\nWe chose Rust.",
            None,
            &labels,
            "alice",
            "claude-code",
        )
        .expect("Failed to create page");

        assert_eq!(page.title, "Test Page");
        assert_eq!(page.page_type, PageType::Decision);
        assert_eq!(page.content, "# Decision\n\nWe chose Rust.");
        assert_eq!(page.created_by_user, "alice");
        assert_eq!(page.created_by_agent, "claude-code");
        assert_eq!(page.version, 1);
        assert_eq!(page.labels, labels);

        let retrieved = get_page(&conn, &page.id).expect("Failed to get page");
        assert_eq!(retrieved.id, page.id);
        assert_eq!(retrieved.labels.len(), 2);
        assert!(retrieved.labels.contains(&"rust".to_string()));
        assert!(retrieved.labels.contains(&"testing".to_string()));
    }

    #[test]
    fn test_get_page_not_found() {
        let conn = setup_test_db();

        let result = get_page(&conn, "nonexistent-id");
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_update_page_title_and_content() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Original Title",
            PageType::Reference,
            "Original content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let updated = update_page(
            &conn,
            &page.id,
            Some("Updated Title"),
            Some("Updated content"),
            None,
            None,
        )
        .expect("Failed to update page");

        assert_eq!(updated.title, "Updated Title");
        assert_eq!(updated.content, "Updated content");
        assert_eq!(updated.version, 2);
        assert_ne!(updated.updated_at, page.updated_at);
    }

    #[test]
    fn test_update_page_with_version_check_success() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Title",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let updated = update_page(&conn, &page.id, Some("New Title"), None, None, Some(1))
            .expect("Failed to update page");

        assert_eq!(updated.title, "New Title");
        assert_eq!(updated.version, 2);
    }

    #[test]
    fn test_update_page_version_conflict() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Title",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // Update once to bump version
        update_page(&conn, &page.id, Some("Updated"), None, None, None).expect("Failed to update page");

        // Try to update with stale version
        let result = update_page(&conn, &page.id, Some("Another Update"), None, None, Some(1));

        match result {
            Err(KbError::VersionConflict { expected, actual }) => {
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            _ => panic!("Expected VersionConflict error"),
        }
    }

    #[test]
    fn test_append_to_page() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Log",
            PageType::SessionLog,
            "First entry",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let updated = append_to_page(&conn, &page.id, "Second entry")
            .expect("Failed to append to page");

        assert_eq!(updated.content, "First entry\nSecond entry");
        assert_eq!(updated.version, 2);
    }

    #[test]
    fn test_append_to_empty_page() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Empty",
            PageType::Reference,
            "",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let updated = append_to_page(&conn, &page.id, "First content")
            .expect("Failed to append to page");

        assert_eq!(updated.content, "First content");
    }

    #[test]
    fn test_list_pages_no_filters() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        create_page(
            &conn,
            &space.id,
            None,
            "Page 1",
            PageType::Reference,
            "Content 1",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        create_page(
            &conn,
            &space.id,
            None,
            "Page 2",
            PageType::Decision,
            "Content 2",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let filters = PageFilters {
            space_id: None,
            page_type: None,
            label: None,
            created_by_user: None,
            created_by_agent: None,
        };

        let pages = list_pages(&conn, &filters).expect("Failed to list pages");
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn test_list_pages_filter_by_space() {
        let conn = setup_test_db();

        let space1 = create_space(&conn, "space1", "Space 1", "").expect("Failed to create space");
        let space2 = create_space(&conn, "space2", "Space 2", "").expect("Failed to create space");

        create_page(
            &conn,
            &space1.id,
            None,
            "Page in Space 1",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        create_page(
            &conn,
            &space2.id,
            None,
            "Page in Space 2",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let filters = PageFilters {
            space_id: Some(space1.id.clone()),
            page_type: None,
            label: None,
            created_by_user: None,
            created_by_agent: None,
        };

        let pages = list_pages(&conn, &filters).expect("Failed to list pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Page in Space 1");
    }

    #[test]
    fn test_list_pages_filter_by_type() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        create_page(
            &conn,
            &space.id,
            None,
            "Decision",
            PageType::Decision,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        create_page(
            &conn,
            &space.id,
            None,
            "Reference",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let filters = PageFilters {
            space_id: None,
            page_type: Some(PageType::Decision),
            label: None,
            created_by_user: None,
            created_by_agent: None,
        };

        let pages = list_pages(&conn, &filters).expect("Failed to list pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_type, PageType::Decision);
    }

    #[test]
    fn test_list_pages_filter_by_label() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        create_page(
            &conn,
            &space.id,
            None,
            "Rust Page",
            PageType::Reference,
            "Content",
            None,
            &vec!["rust".to_string()],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        create_page(
            &conn,
            &space.id,
            None,
            "Python Page",
            PageType::Reference,
            "Content",
            None,
            &vec!["python".to_string()],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let filters = PageFilters {
            space_id: None,
            page_type: None,
            label: Some("rust".to_string()),
            created_by_user: None,
            created_by_agent: None,
        };

        let pages = list_pages(&conn, &filters).expect("Failed to list pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Rust Page");
    }

    #[test]
    fn test_list_pages_filter_by_user_and_agent() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        create_page(
            &conn,
            &space.id,
            None,
            "Alice's Page",
            PageType::Reference,
            "Content",
            None,
            &[],
            "alice",
            "claude-code",
        )
        .expect("Failed to create page");
        create_page(
            &conn,
            &space.id,
            None,
            "Bob's Page",
            PageType::Reference,
            "Content",
            None,
            &[],
            "bob",
            "cursor",
        )
        .expect("Failed to create page");

        let filters = PageFilters {
            space_id: None,
            page_type: None,
            label: None,
            created_by_user: Some("alice".to_string()),
            created_by_agent: Some("claude-code".to_string()),
        };

        let pages = list_pages(&conn, &filters).expect("Failed to list pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Alice's Page");
    }

    #[test]
    fn test_delete_page() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "To Delete",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        delete_page(&conn, &page.id).expect("Failed to delete page");

        let result = get_page(&conn, &page.id);
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_delete_page_cascades_labels() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Page",
            PageType::Reference,
            "Content",
            None,
            &vec!["label1".to_string(), "label2".to_string()],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        delete_page(&conn, &page.id).expect("Failed to delete page");

        // Verify labels are gone
        let labels = get_labels(&conn, &page.id).expect("Failed to get labels");
        assert_eq!(labels.len(), 0);
    }

    #[test]
    fn test_set_labels_replaces_existing() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Page",
            PageType::Reference,
            "Content",
            None,
            &vec!["old1".to_string(), "old2".to_string()],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        set_labels(&conn, &page.id, &vec!["new1".to_string(), "new2".to_string()])
            .expect("Failed to set labels");

        let labels = get_labels(&conn, &page.id).expect("Failed to get labels");
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"new1".to_string()));
        assert!(labels.contains(&"new2".to_string()));
        assert!(!labels.contains(&"old1".to_string()));
    }

    #[test]
    fn test_create_and_list_links() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page1 = create_page(
            &conn,
            &space.id,
            None,
            "Page 1",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        let page2 = create_page(
            &conn,
            &space.id,
            None,
            "Page 2",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let link = create_link(&conn, &page1.id, &page2.id, LinkRelation::RelatesTo)
            .expect("Failed to create link");

        assert_eq!(link.source_id, page1.id);
        assert_eq!(link.target_id, page2.id);
        assert_eq!(link.relation, LinkRelation::RelatesTo);

        let links = list_links(&conn, &page1.id).expect("Failed to list links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].source_id, page1.id);
        assert_eq!(links[0].target_id, page2.id);

        // Verify bidirectional listing
        let links_from_page2 = list_links(&conn, &page2.id).expect("Failed to list links");
        assert_eq!(links_from_page2.len(), 1);
    }

    #[test]
    fn test_create_link_with_different_relations() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page1 = create_page(
            &conn,
            &space.id,
            None,
            "Page 1",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        let page2 = create_page(
            &conn,
            &space.id,
            None,
            "Page 2",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        let link = create_link(&conn, &page1.id, &page2.id, LinkRelation::Supersedes)
            .expect("Failed to create link");

        assert_eq!(link.relation, LinkRelation::Supersedes);
    }

    #[test]
    fn test_delete_link() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page1 = create_page(
            &conn,
            &space.id,
            None,
            "Page 1",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        let page2 = create_page(
            &conn,
            &space.id,
            None,
            "Page 2",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        create_link(&conn, &page1.id, &page2.id, LinkRelation::RelatesTo)
            .expect("Failed to create link");

        delete_link(&conn, &page1.id, &page2.id).expect("Failed to delete link");

        let links = list_links(&conn, &page1.id).expect("Failed to list links");
        assert_eq!(links.len(), 0);
    }

    #[test]
    fn test_delete_link_not_found() {
        let conn = setup_test_db();

        let result = delete_link(&conn, "nonexistent-source", "nonexistent-target");
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_delete_page_cascades_links() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page1 = create_page(
            &conn,
            &space.id,
            None,
            "Page 1",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        let page2 = create_page(
            &conn,
            &space.id,
            None,
            "Page 2",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");
        let page3 = create_page(
            &conn,
            &space.id,
            None,
            "Page 3",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        create_link(&conn, &page1.id, &page2.id, LinkRelation::RelatesTo)
            .expect("Failed to create link");
        create_link(&conn, &page2.id, &page3.id, LinkRelation::Elaborates)
            .expect("Failed to create link");

        delete_page(&conn, &page2.id).expect("Failed to delete page");

        // Verify links involving page2 are gone
        let links_from_page1 = list_links(&conn, &page1.id).expect("Failed to list links");
        assert_eq!(links_from_page1.len(), 0);

        let links_from_page3 = list_links(&conn, &page3.id).expect("Failed to list links");
        assert_eq!(links_from_page3.len(), 0);
    }

    #[test]
    fn test_update_page_atomic_version_check() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Original",
            PageType::Reference,
            "Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // This should succeed and return version 2
        let updated = update_page(&conn, &page.id, Some("First Update"), None, None, Some(1))
            .expect("First update should succeed");
        assert_eq!(updated.title, "First Update");
        assert_eq!(updated.version, 2);

        // This should fail because version is now 2, not 1
        let result = update_page(&conn, &page.id, Some("Second Update"), None, None, Some(1));
        match result {
            Err(KbError::VersionConflict { expected, actual }) => {
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            _ => panic!("Expected VersionConflict error"),
        }

        // Verify the page still has the first update (atomic operation prevented race)
        let current = get_page(&conn, &page.id).expect("Should get page");
        assert_eq!(current.title, "First Update");
        assert_eq!(current.version, 2);
    }

    #[test]
    fn test_update_page_not_found() {
        let conn = setup_test_db();

        let result = update_page(&conn, "nonexistent-id", Some("Title"), None, None, None);
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_update_page_partial_updates_with_coalesce() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Original Title",
            PageType::Reference,
            "Original Content",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // Update only title, keep content
        let updated = update_page(&conn, &page.id, Some("New Title"), None, None, None)
            .expect("Update should succeed");
        assert_eq!(updated.title, "New Title");
        assert_eq!(updated.content, "Original Content");

        // Update only content, keep title
        let updated = update_page(&conn, &page.id, None, Some("New Content"), None, None)
            .expect("Update should succeed");
        assert_eq!(updated.title, "New Title");
        assert_eq!(updated.content, "New Content");
    }

    #[test]
    fn test_append_to_page_atomic() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Log",
            PageType::SessionLog,
            "First entry",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // Append should work atomically
        let updated = append_to_page(&conn, &page.id, "Second entry")
            .expect("Append should succeed");
        assert_eq!(updated.content, "First entry\nSecond entry");
        assert_eq!(updated.version, 2);

        // Another append
        let updated = append_to_page(&conn, &page.id, "Third entry")
            .expect("Append should succeed");
        assert_eq!(updated.content, "First entry\nSecond entry\nThird entry");
        assert_eq!(updated.version, 3);
    }

    #[test]
    fn test_append_to_page_not_found() {
        let conn = setup_test_db();

        let result = append_to_page(&conn, "nonexistent-id", "content");
        assert!(matches!(result, Err(KbError::NotFound(_))));
    }

    #[test]
    fn test_update_page_prevents_toctou_race_condition() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Original",
            PageType::Reference,
            "Content v1",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // Simulate what would happen in a race condition:
        // 1. First agent tries to update with version check
        let update1 = update_page(&conn, &page.id, Some("Update 1"), None, None, Some(1));
        assert!(update1.is_ok());
        assert_eq!(update1.unwrap().version, 2);

        // 2. Second agent tries to update with stale version (simulating TOCTOU)
        // This MUST fail because the version is now 2, not 1
        let update2 = update_page(&conn, &page.id, Some("Update 2"), None, None, Some(1));
        match update2 {
            Err(KbError::VersionConflict { expected, actual }) => {
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            _ => panic!("Expected VersionConflict, got: {:?}", update2),
        }

        // Verify the page has the first update only
        let final_page = get_page(&conn, &page.id).expect("Should get page");
        assert_eq!(final_page.title, "Update 1");
        assert_eq!(final_page.version, 2);
    }

    #[test]
    fn test_append_to_page_prevents_lost_updates() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Log",
            PageType::SessionLog,
            "Entry 1",
            None,
            &[],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // Multiple appends should all succeed atomically
        let result1 = append_to_page(&conn, &page.id, "Entry 2");
        assert!(result1.is_ok());
        assert_eq!(result1.unwrap().version, 2);

        let result2 = append_to_page(&conn, &page.id, "Entry 3");
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap().version, 3);

        let result3 = append_to_page(&conn, &page.id, "Entry 4");
        assert!(result3.is_ok());
        assert_eq!(result3.unwrap().version, 4);

        // Verify all appends are present
        let final_page = get_page(&conn, &page.id).expect("Should get page");
        assert_eq!(final_page.content, "Entry 1\nEntry 2\nEntry 3\nEntry 4");
        assert_eq!(final_page.version, 4);
    }

    #[test]
    fn test_create_page_with_duplicate_labels_fails_atomically() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");

        // Try to create a page with duplicate labels (violates PRIMARY KEY constraint)
        let duplicate_labels = vec!["rust".to_string(), "rust".to_string()];
        let result = create_page(
            &conn,
            &space.id,
            None,
            "Test Page",
            PageType::Reference,
            "Content",
            None,
            &duplicate_labels,
            "user",
            "agent",
        );

        // Should fail due to constraint violation
        assert!(matches!(result, Err(KbError::Db(_))));

        // Verify that the page was NOT created (transaction rolled back)
        let pages = list_pages(&conn, &PageFilters {
            space_id: Some(space.id.clone()),
            page_type: None,
            label: None,
            created_by_user: None,
            created_by_agent: None,
        })
        .expect("Failed to list pages");

        assert_eq!(pages.len(), 0, "No pages should exist after failed transaction");
    }

    #[test]
    fn test_set_labels_with_duplicate_fails_atomically() {
        let conn = setup_test_db();

        let space = create_space(&conn, "test-space", "Test", "").expect("Failed to create space");
        let page = create_page(
            &conn,
            &space.id,
            None,
            "Test Page",
            PageType::Reference,
            "Content",
            None,
            &vec!["original".to_string()],
            "user",
            "agent",
        )
        .expect("Failed to create page");

        // Verify original label exists
        let labels = get_labels(&conn, &page.id).expect("Failed to get labels");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0], "original");

        // Try to set labels with duplicates (violates PRIMARY KEY constraint)
        let duplicate_labels = vec!["new".to_string(), "new".to_string()];
        let result = set_labels(&conn, &page.id, &duplicate_labels);

        // Should fail due to constraint violation
        assert!(matches!(result, Err(KbError::Db(_))));

        // Verify that the original label is still there (transaction rolled back)
        let labels = get_labels(&conn, &page.id).expect("Failed to get labels");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0], "original", "Original label should still exist after failed set_labels");
    }

    #[test]
    fn test_list_top_level_pages() {
        let conn = setup_test_db();
        let space = create_space(&conn, "s", "S", "").expect("create space");

        let p1 = create_page(&conn, &space.id, None, "Bravo", PageType::Reference, "", None, &[], "u", "a")
            .expect("create p1");
        let _p2 = create_page(&conn, &space.id, Some(&p1.id), "Child", PageType::Reference, "", None, &[], "u", "a")
            .expect("create p2");
        let _p3 = create_page(&conn, &space.id, None, "Alpha", PageType::Decision, "", None, &["lbl".to_string()], "u", "a")
            .expect("create p3");

        let top = list_top_level_pages(&conn, &space.id).expect("list top-level");
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].title, "Alpha");
        assert_eq!(top[1].title, "Bravo");
        assert!(top[0].labels.contains(&"lbl".to_string()));
    }

    #[test]
    fn test_list_child_pages() {
        let conn = setup_test_db();
        let space = create_space(&conn, "s", "S", "").expect("create space");

        let parent = create_page(&conn, &space.id, None, "Parent", PageType::Reference, "", None, &[], "u", "a")
            .expect("create parent");
        let _c1 = create_page(&conn, &space.id, Some(&parent.id), "Zebra", PageType::Reference, "", None, &[], "u", "a")
            .expect("create c1");
        let _c2 = create_page(&conn, &space.id, Some(&parent.id), "Apple", PageType::Decision, "", None, &[], "u", "a")
            .expect("create c2");

        let children = list_child_pages(&conn, &parent.id).expect("list children");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].title, "Apple");
        assert_eq!(children[1].title, "Zebra");
    }

    #[test]
    fn test_has_children() {
        let conn = setup_test_db();
        let space = create_space(&conn, "s", "S", "").expect("create space");

        let parent = create_page(&conn, &space.id, None, "Parent", PageType::Reference, "", None, &[], "u", "a")
            .expect("create parent");
        let leaf = create_page(&conn, &space.id, None, "Leaf", PageType::Reference, "", None, &[], "u", "a")
            .expect("create leaf");
        let _child = create_page(&conn, &space.id, Some(&parent.id), "Child", PageType::Reference, "", None, &[], "u", "a")
            .expect("create child");

        assert!(has_children(&conn, &parent.id).expect("check parent"));
        assert!(!has_children(&conn, &leaf.id).expect("check leaf"));
    }

    #[test]
    fn test_create_page_with_sections() {
        let conn = setup_test_db();
        let space = create_space(&conn, "test-space", "Test", "").expect("create space");
        let sections = serde_json::json!({
            "context": "We need a DB.",
            "decision": "Use SQLite.",
            "options_considered": "Postgres, SQLite",
            "consequences": "Single-file storage."
        });
        let page = create_page(
            &conn, &space.id, None, "DB Choice", PageType::Decision,
            "", Some(&sections), &[], "user", "agent",
        ).expect("create page with sections");
        assert!(page.sections.is_some());
        assert!(page.content.contains("## Context"));
        assert!(page.content.contains("## Decision"));
    }

    #[test]
    fn test_get_page_returns_sections() {
        let conn = setup_test_db();
        let space = create_space(&conn, "test-space", "Test", "").expect("create space");
        let sections = serde_json::json!({"context": "test", "decision": "test"});
        let page = create_page(
            &conn, &space.id, None, "Test", PageType::Decision,
            "", Some(&sections), &[], "user", "agent",
        ).expect("create");
        let retrieved = get_page(&conn, &page.id).expect("get");
        assert!(retrieved.sections.is_some());
        let secs = retrieved.sections.unwrap();
        assert_eq!(secs["context"], "test");
    }

    #[test]
    fn test_update_page_with_sections() {
        let conn = setup_test_db();
        let space = create_space(&conn, "test-space", "Test", "").expect("create space");
        let page = create_page(
            &conn, &space.id, None, "Test", PageType::Decision,
            "old content", None, &[], "user", "agent",
        ).expect("create");
        let new_sections = serde_json::json!({"context": "updated", "decision": "new choice"});
        let updated = update_page(&conn, &page.id, None, None, Some(&new_sections), None)
            .expect("update");
        assert!(updated.sections.is_some());
        assert!(updated.content.contains("## Context"));
    }

    #[test]
    fn test_add_label_new() {
        let conn = setup_test_db();
        let space = create_space(&conn, "s", "S", "").expect("create space");
        let page = create_page(
            &conn, &space.id, None, "Page", PageType::Reference, "", None,
            &[], "u", "a",
        ).expect("create page");

        add_label(&conn, &page.id, "new-label").expect("add label");
        let labels = get_labels(&conn, &page.id).expect("get labels");
        assert_eq!(labels, vec!["new-label"]);
    }

    #[test]
    fn test_add_label_duplicate_is_noop() {
        let conn = setup_test_db();
        let space = create_space(&conn, "s", "S", "").expect("create space");
        let page = create_page(
            &conn, &space.id, None, "Page", PageType::Reference, "", None,
            &["existing".to_string()], "u", "a",
        ).expect("create page");

        // Adding the same label again should not error
        add_label(&conn, &page.id, "existing").expect("add duplicate label");
        let labels = get_labels(&conn, &page.id).expect("get labels");
        assert_eq!(labels, vec!["existing"]);
    }

    #[test]
    fn test_add_label_preserves_existing() {
        let conn = setup_test_db();
        let space = create_space(&conn, "s", "S", "").expect("create space");
        let page = create_page(
            &conn, &space.id, None, "Page", PageType::Reference, "", None,
            &["alpha".to_string(), "beta".to_string()], "u", "a",
        ).expect("create page");

        add_label(&conn, &page.id, "gamma").expect("add label");
        let labels = get_labels(&conn, &page.id).expect("get labels");
        assert_eq!(labels, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn test_list_pages_includes_sections() {
        let conn = setup_test_db();
        let space = create_space(&conn, "test-space", "Test", "").expect("create space");
        let sections = serde_json::json!({"context": "test"});
        create_page(
            &conn, &space.id, None, "Test", PageType::Decision,
            "", Some(&sections), &[], "user", "agent",
        ).expect("create");
        let filters = PageFilters {
            space_id: Some(space.id), page_type: None, label: None,
            created_by_user: None, created_by_agent: None,
        };
        let pages = list_pages(&conn, &filters).expect("list");
        assert_eq!(pages.len(), 1);
        assert!(pages[0].sections.is_some());
    }
}
