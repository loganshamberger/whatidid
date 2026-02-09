//! Database connection management, migrations, and error types.
//!
//! This module handles all SQLite connection setup with appropriate settings
//! for concurrent access (WAL mode, foreign keys, busy timeout), schema
//! versioning via migrations, and a unified error type for the entire crate.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

/// Central error type for the knowledge base application.
///
/// Uses `thiserror` for automatic `Error` trait implementation and `Display`
/// formatting. Includes conversions from common error types via `From` impls.
#[derive(Debug, Error)]
pub enum KbError {
    /// Database operation failed.
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    /// I/O operation failed (file/directory creation, reading migrations, etc).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Requested entity was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Optimistic concurrency control: version mismatch on update.
    #[error("Version conflict: expected {expected}, but current version is {actual}")]
    VersionConflict { expected: i64, actual: i64 },

    /// Invalid input provided by the user or caller.
    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

/// Returns the path to the SQLite database file.
///
/// Resolution order:
/// 1. `KB_PATH` environment variable (if set)
/// 2. `~/.knowledge-base/kb.db` (default)
///
/// Creates the parent directory if it doesn't exist.
///
/// # Errors
///
/// Returns `KbError::Io` if:
/// - Home directory cannot be determined (when `KB_PATH` is not set)
/// - Parent directory creation fails
///
/// # Examples
///
/// ```no_run
/// use kb::db::db_path;
///
/// let path = db_path().expect("Failed to get database path");
/// println!("Database at: {:?}", path);
/// ```
pub fn db_path() -> Result<PathBuf, KbError> {
    let path = if let Ok(kb_path) = std::env::var("KB_PATH") {
        PathBuf::from(kb_path)
    } else {
        let home = dirs::home_dir().ok_or_else(|| {
            KbError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;
        home.join(".knowledge-base").join("kb.db")
    };

    // Ensure the parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    Ok(path)
}

/// Opens a SQLite connection to the default database with proper settings.
///
/// Configured for multi-agent concurrent access:
/// - **WAL mode**: Allows concurrent readers with serialized writers
/// - **Foreign keys**: Enabled for referential integrity
/// - **Busy timeout**: 5 seconds to handle write contention gracefully
///
/// # Errors
///
/// Returns `KbError::Db` if the connection cannot be opened or configured.
///
/// # Examples
///
/// ```no_run
/// use kb::db::open_connection;
///
/// let conn = open_connection().expect("Failed to open connection");
/// ```
pub fn open_connection() -> Result<Connection, KbError> {
    let path = db_path()?;
    open_connection_at(&path)
}

/// Opens a SQLite connection at the specified path with proper settings.
///
/// This function is identical to `open_connection()` but accepts an explicit path.
/// Primarily used for testing with temporary databases.
///
/// Configured for multi-agent concurrent access:
/// - **WAL mode**: Allows concurrent readers with serialized writers
/// - **Foreign keys**: Enabled for referential integrity
/// - **Busy timeout**: 5 seconds to handle write contention gracefully
///
/// # Errors
///
/// Returns `KbError::Db` if the connection cannot be opened or configured.
///
/// # Examples
///
/// ```no_run
/// use kb::db::open_connection_at;
/// use std::path::Path;
///
/// let conn = open_connection_at(Path::new("/tmp/test.db"))
///     .expect("Failed to open connection");
/// ```
pub fn open_connection_at(path: &std::path::Path) -> Result<Connection, KbError> {
    let conn = Connection::open(path)?;

    // Enable WAL mode for concurrent reads
    conn.pragma_update(None, "journal_mode", "WAL")?;

    // Enable foreign key constraints
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // Set busy timeout to 5 seconds for multi-agent concurrency
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    Ok(conn)
}

/// Runs all pending database migrations.
///
/// Migrations are applied transactionally and idempotently:
/// 1. Reads the current schema version from `schema_meta` table (0 if table doesn't exist)
/// 2. Loads embedded migration SQL files
/// 3. Runs each migration with version > current version, in order
/// 4. Each migration runs in its own transaction
///
/// If any migration fails, the transaction is rolled back and an error is returned.
///
/// # Migration Files
///
/// Migrations are embedded in the binary using `include_str!`. They must be named
/// `NNN_description.sql` where NNN is a zero-padded number (e.g., `001_initial.sql`).
///
/// Each migration should update `schema_meta.version` to its target version.
///
/// # Errors
///
/// Returns `KbError::Db` if:
/// - Version check query fails
/// - Migration SQL execution fails
/// - Transaction commit fails
///
/// # Examples
///
/// ```no_run
/// use kb::db::{open_connection, run_migrations};
///
/// let mut conn = open_connection().expect("Failed to open connection");
/// run_migrations(&mut conn).expect("Failed to run migrations");
/// ```
pub fn run_migrations(conn: &mut Connection) -> Result<(), KbError> {
    // Determine current schema version
    let current_version: i64 = conn
        .query_row(
            "SELECT version FROM schema_meta LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0); // Fresh database starts at version 0

    // Embedded migrations
    let migrations: Vec<(i64, &str)> = vec![
        (1, include_str!("../migrations/001_initial.sql")),
        (2, include_str!("../migrations/002_sections.sql")),
        (3, include_str!("../migrations/003_timestamps.sql")),
    ];

    // Run pending migrations
    for (target_version, sql) in migrations {
        if target_version > current_version {
            // Run each migration in a transaction
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.commit()?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_db_path_respects_kb_path_env_var() {
        let temp_dir = std::env::temp_dir();
        let custom_path = temp_dir.join("custom_kb_test.db");

        // Set the environment variable
        env::set_var("KB_PATH", custom_path.to_str().unwrap());

        let result = db_path().expect("db_path should succeed");

        // Clean up
        env::remove_var("KB_PATH");

        assert_eq!(result, custom_path);
    }

    #[test]
    fn test_db_path_falls_back_to_default() {
        // Ensure KB_PATH is not set
        env::remove_var("KB_PATH");

        let result = db_path().expect("db_path should succeed with default");

        // Should contain .knowledge-base/kb.db
        assert!(result.to_string_lossy().contains(".knowledge-base"));
        assert!(result.to_string_lossy().ends_with("kb.db"));
    }

    #[test]
    fn test_migration_creates_tables_from_scratch() {
        // Create an in-memory database
        let mut conn = Connection::open_in_memory().expect("Failed to open in-memory DB");

        // Configure the connection with the same settings as open_connection_at
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("Failed to enable foreign keys");

        // Run migrations
        run_migrations(&mut conn).expect("Migrations should succeed");

        // Verify schema_meta table exists and has version 3
        let version: i64 = conn
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .expect("schema_meta should exist");
        assert_eq!(version, 3);

        // Verify main tables exist
        let table_names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("Failed to prepare query")
            .query_map([], |row| row.get(0))
            .expect("Failed to query tables")
            .collect::<Result<Vec<_>, _>>()
            .expect("Failed to collect table names");

        assert!(table_names.contains(&"schema_meta".to_string()));
        assert!(table_names.contains(&"spaces".to_string()));
        assert!(table_names.contains(&"pages".to_string()));
        assert!(table_names.contains(&"labels".to_string()));
        assert!(table_names.contains(&"links".to_string()));
        assert!(table_names.contains(&"pages_fts".to_string()));
    }

    #[test]
    fn test_migration_is_idempotent() {
        // Create an in-memory database
        let mut conn = Connection::open_in_memory().expect("Failed to open in-memory DB");

        // Configure the connection
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("Failed to enable foreign keys");

        // Run migrations first time
        run_migrations(&mut conn).expect("First migration should succeed");

        let version_after_first: i64 = conn
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .expect("schema_meta should exist after first migration");

        // Run migrations second time
        run_migrations(&mut conn).expect("Second migration should succeed");

        let version_after_second: i64 = conn
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .expect("schema_meta should exist after second migration");

        // Version should be unchanged
        assert_eq!(version_after_first, version_after_second);
        assert_eq!(version_after_second, 3);

        // Verify we can still query tables (no corruption)
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM spaces", [], |row| row.get(0))
            .expect("Should be able to query spaces table");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_open_connection_at_configures_correctly() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_config.db");

        // Clean up any existing test database
        let _ = std::fs::remove_file(&db_path);

        let conn = open_connection_at(&db_path).expect("Should open connection");

        // Verify WAL mode
        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("Should query journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        // Verify foreign keys are enabled
        let foreign_keys: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .expect("Should query foreign_keys");
        assert_eq!(foreign_keys, 1);

        // Clean up
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
    }
}
