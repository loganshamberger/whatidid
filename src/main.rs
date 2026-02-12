//! Knowledge Base CLI — a local knowledge management tool for AI agents.
//!
//! This binary provides the `kb` command with subcommands for managing spaces,
//! pages, links, and search. All output is JSON by default (for agent consumption)
//! with an optional `--pretty` flag for human readability.

mod db;
mod models;
mod output;
mod repo;
mod search;
mod tui;

use clap::{Parser, Subcommand};
use models::{AgentIdentity, LinkRelation, PageType};
use output::OutputMode;
use std::io::{self, Read as _};
use std::process;

/// Input validation for security hardening.
mod validation {
    use crate::db::KbError;

    pub const MAX_SLUG_LEN: usize = 128;
    pub const MAX_TITLE_LEN: usize = 500;
    pub const MAX_BODY_LEN: usize = 10_000_000; // 10 MB
    pub const MAX_LABEL_LEN: usize = 100;
    pub const MAX_LABELS_COUNT: usize = 50;
    pub const MAX_SECTIONS_LEN: usize = 10_000_000; // 10 MB
    pub const MAX_NAME_LEN: usize = 256;
    pub const MAX_DESCRIPTION_LEN: usize = 2000;

    pub fn validate_slug(slug: &str) -> Result<(), KbError> {
        if slug.is_empty() {
            return Err(KbError::InvalidInput("Slug must not be empty".to_string()));
        }
        if slug.len() > MAX_SLUG_LEN {
            return Err(KbError::InvalidInput(format!("Slug too long (max {} characters)", MAX_SLUG_LEN)));
        }
        if !slug.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(KbError::InvalidInput("Slug must start with a lowercase letter".to_string()));
        }
        if !slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
            return Err(KbError::InvalidInput(
                "Slug must contain only lowercase letters, digits, hyphens, and underscores".to_string()
            ));
        }
        Ok(())
    }

    pub fn validate_title(title: &str) -> Result<(), KbError> {
        if title.is_empty() {
            return Err(KbError::InvalidInput("Title must not be empty".to_string()));
        }
        if title.len() > MAX_TITLE_LEN {
            return Err(KbError::InvalidInput(format!("Title too long (max {} characters)", MAX_TITLE_LEN)));
        }
        Ok(())
    }

    pub fn validate_body(body: &str) -> Result<(), KbError> {
        if body.len() > MAX_BODY_LEN {
            return Err(KbError::InvalidInput(format!("Body too long (max {} bytes)", MAX_BODY_LEN)));
        }
        Ok(())
    }

    pub fn validate_labels(labels: &[String]) -> Result<(), KbError> {
        if labels.len() > MAX_LABELS_COUNT {
            return Err(KbError::InvalidInput(format!("Too many labels (max {})", MAX_LABELS_COUNT)));
        }
        for label in labels {
            if label.len() > MAX_LABEL_LEN {
                return Err(KbError::InvalidInput(
                    format!("Label '{}' too long (max {} characters)", label, MAX_LABEL_LEN)
                ));
            }
        }
        Ok(())
    }

    pub fn validate_sections_json(json_str: &str) -> Result<(), KbError> {
        if json_str.len() > MAX_SECTIONS_LEN {
            return Err(KbError::InvalidInput(format!("Sections JSON too long (max {} bytes)", MAX_SECTIONS_LEN)));
        }
        Ok(())
    }

    pub fn validate_name(name: &str) -> Result<(), KbError> {
        if name.len() > MAX_NAME_LEN {
            return Err(KbError::InvalidInput(format!("Name too long (max {} characters)", MAX_NAME_LEN)));
        }
        Ok(())
    }

    pub fn validate_description(desc: &str) -> Result<(), KbError> {
        if desc.len() > MAX_DESCRIPTION_LEN {
            return Err(KbError::InvalidInput(format!("Description too long (max {} characters)", MAX_DESCRIPTION_LEN)));
        }
        Ok(())
    }
}

/// A local knowledge base CLI for AI agents.
///
/// Manages structured knowledge documents organized into spaces.
/// Designed to be invoked by AI agent frameworks as a subprocess.
/// All output is JSON by default; use --pretty for human-readable format.
#[derive(Parser)]
#[command(name = "whatidid", version, about)]
struct Cli {
    /// Output in human-readable format instead of JSON.
    #[arg(long, global = true)]
    pretty: bool,

    /// Override the user identity (default: $KB_USER or $USER).
    #[arg(long, global = true)]
    user: Option<String>,

    /// Override the agent identity (default: $KB_AGENT).
    #[arg(long, global = true)]
    agent: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage spaces (top-level organizational units).
    Space {
        #[command(subcommand)]
        action: SpaceAction,
    },
    /// Manage pages (knowledge documents).
    Page {
        #[command(subcommand)]
        action: PageAction,
    },
    /// Full-text search with optional structured filters.
    Search {
        /// Search query text.
        query: Option<String>,
        /// Filter by space slug.
        #[arg(long)]
        space: Option<String>,
        /// Filter by page type (decision, architecture, session-log, reference, troubleshooting, runbook).
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by label.
        #[arg(long)]
        label: Option<String>,
        /// Filter by creating agent.
        #[arg(long)]
        created_by_agent: Option<String>,
        /// Filter to pages containing a specific section key.
        #[arg(long)]
        section: Option<String>,
    },
    /// Manage links between pages.
    Link {
        #[command(subcommand)]
        action: LinkAction,
    },
    /// Interactive TUI browser for exploring spaces and pages.
    Browse,
}

#[derive(Subcommand)]
enum SpaceAction {
    /// Create a new space.
    Create {
        /// URL-friendly slug identifier (e.g., "my-project").
        slug: String,
        /// Display name for the space.
        #[arg(long)]
        name: Option<String>,
        /// Description of the space.
        #[arg(long, default_value = "")]
        description: String,
    },
    /// List all spaces.
    List,
    /// Get a space by slug.
    Get {
        /// The space slug.
        slug: String,
    },
    /// Delete a space (must have no pages).
    Delete {
        /// The space slug.
        slug: String,
    },
}

#[derive(Subcommand)]
enum PageAction {
    /// Create a new page.
    Create {
        /// Space slug this page belongs to.
        #[arg(long)]
        space: String,
        /// Page title.
        #[arg(long)]
        title: String,
        /// Page type (decision, architecture, session-log, reference, troubleshooting, runbook).
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
        /// Parent page ID for hierarchical nesting.
        #[arg(long)]
        parent: Option<String>,
        /// Comma-separated labels.
        #[arg(long)]
        labels: Option<String>,
        /// Page content (markdown). Omit to read from stdin.
        #[arg(long)]
        body: Option<String>,
        /// Read body from stdin.
        #[arg(long)]
        stdin: bool,
        /// Structured sections as JSON (e.g. '{"context":"...", "decision":"..."}').
        /// Mutually exclusive with --body.
        #[arg(long)]
        sections: Option<String>,
    },
    /// Get a page by ID.
    Get {
        /// The page ID.
        id: String,
    },
    /// Update a page's title and/or content.
    Update {
        /// The page ID.
        id: String,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New content.
        #[arg(long)]
        body: Option<String>,
        /// Read body from stdin.
        #[arg(long)]
        stdin: bool,
        /// Structured sections as JSON. Replaces existing sections.
        #[arg(long)]
        sections: Option<String>,
        /// Expected version for optimistic concurrency control.
        #[arg(long)]
        version: Option<i64>,
        /// Comma-separated labels. Replaces all existing labels.
        #[arg(long)]
        labels: Option<String>,
    },
    /// Append content to an existing page.
    Append {
        /// The page ID.
        id: String,
        /// Content to append.
        #[arg(long)]
        body: Option<String>,
        /// Read body from stdin.
        #[arg(long)]
        stdin: bool,
    },
    /// List pages with optional filters.
    List {
        /// Filter by space slug.
        #[arg(long)]
        space: Option<String>,
        /// Filter by page type.
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by label.
        #[arg(long)]
        label: Option<String>,
        /// Filter by creating user.
        #[arg(long)]
        created_by_user: Option<String>,
        /// Filter by creating agent.
        #[arg(long)]
        created_by_agent: Option<String>,
    },
    /// Delete a page by ID.
    Delete {
        /// The page ID.
        id: String,
    },
    /// Show the expected sections schema for a page type.
    Schema {
        /// Page type to show schema for.
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
    },
}

#[derive(Subcommand)]
enum LinkAction {
    /// Create a link between two pages.
    Create {
        /// Source page ID.
        source: String,
        /// Target page ID.
        target: String,
        /// Relationship type (relates-to, supersedes, depends-on, elaborates).
        #[arg(long, default_value = "relates-to")]
        relation: String,
    },
    /// List all links for a page.
    List {
        /// Page ID to list links for.
        page_id: String,
    },
    /// Delete a link between two pages.
    Delete {
        /// Source page ID.
        source: String,
        /// Target page ID.
        target: String,
    },
}

/// Resolve the agent identity from CLI flags, env vars, and system defaults.
fn resolve_identity(cli: &Cli) -> AgentIdentity {
    let user = cli
        .user
        .clone()
        .or_else(|| std::env::var("KB_USER").ok())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "unknown".to_string());

    let agent = cli
        .agent
        .clone()
        .or_else(|| std::env::var("KB_AGENT").ok())
        .unwrap_or_else(|| "unknown".to_string());

    AgentIdentity { user, agent }
}

/// Read body content from --body flag or --stdin.
fn read_body(body: &Option<String>, stdin: bool) -> Result<String, db::KbError> {
    if stdin {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(db::KbError::Io)?;
        Ok(buf)
    } else if let Some(b) = body {
        Ok(b.clone())
    } else {
        Ok(String::new())
    }
}

/// Resolve a space slug to its ID.
fn resolve_space_id(conn: &rusqlite::Connection, slug: &str) -> Result<String, db::KbError> {
    let space = repo::get_space_by_slug(conn, slug)?;
    Ok(space.id)
}

/// Parse a page type string, returning InvalidInput on failure.
fn parse_page_type(s: &str) -> Result<PageType, db::KbError> {
    PageType::from_str(s).ok_or_else(|| {
        db::KbError::InvalidInput(format!(
            "Unknown page type '{}'. Valid types: decision, architecture, session-log, reference, troubleshooting, runbook",
            s
        ))
    })
}

/// Parse a link relation string, returning InvalidInput on failure.
fn parse_link_relation(s: &str) -> Result<LinkRelation, db::KbError> {
    LinkRelation::from_str(s).ok_or_else(|| {
        db::KbError::InvalidInput(format!(
            "Unknown relation '{}'. Valid relations: relates-to, supersedes, depends-on, elaborates",
            s
        ))
    })
}

fn run() -> Result<(), db::KbError> {
    let cli = Cli::parse();
    let mode = if cli.pretty {
        OutputMode::Pretty
    } else {
        OutputMode::Json
    };

    // Open database and run migrations.
    let mut conn = db::open_connection()?;
    db::run_migrations(&mut conn)?;

    match &cli.command {
        // =====================================================================
        // Space commands
        // =====================================================================
        Commands::Space { action } => match action {
            SpaceAction::Create {
                slug,
                name,
                description,
            } => {
                validation::validate_slug(slug)?;
                if let Some(ref n) = name {
                    validation::validate_name(n)?;
                }
                validation::validate_description(description)?;
                let display_name = name.as_deref().unwrap_or(slug);
                let space = repo::create_space(&conn, slug, display_name, description)?;
                output::print(mode, &space, || output::print_pretty_space(&space));
            }
            SpaceAction::List => {
                let spaces = repo::list_spaces(&conn)?;
                output::print(mode, &spaces, || {
                    if spaces.is_empty() {
                        println!("(no spaces)");
                    } else {
                        for s in &spaces {
                            output::print_pretty_space(s);
                            println!();
                        }
                    }
                });
            }
            SpaceAction::Get { slug } => {
                let space = repo::get_space_by_slug(&conn, slug)?;
                output::print(mode, &space, || output::print_pretty_space(&space));
            }
            SpaceAction::Delete { slug } => {
                repo::delete_space(&conn, slug)?;
                let msg = serde_json::json!({"deleted": slug});
                output::print(mode, &msg, || println!("Deleted space '{}'", slug));
            }
        },

        // =====================================================================
        // Page commands
        // =====================================================================
        Commands::Page { action } => match action {
            PageAction::Create {
                space,
                title,
                r#type,
                parent,
                labels,
                body,
                stdin,
                sections,
            } => {
                let identity = resolve_identity(&cli);
                let space_id = resolve_space_id(&conn, space)?;
                let page_type = parse_page_type(r#type)?;
                validation::validate_title(title)?;

                // Parse and validate sections JSON if provided
                if let Some(ref s) = sections {
                    validation::validate_sections_json(s)?;
                }
                let sections_value: Option<serde_json::Value> = match sections {
                    Some(ref s) => {
                        let val: serde_json::Value = serde_json::from_str(s)
                            .map_err(|e| db::KbError::InvalidInput(format!("Invalid sections JSON: {}", e)))?;
                        Some(val)
                    }
                    None => None,
                };

                // Enforce mutual exclusivity: --sections and --body/--stdin
                if sections_value.is_some() && (body.is_some() || *stdin) {
                    return Err(db::KbError::InvalidInput(
                        "--sections is mutually exclusive with --body and --stdin".to_string(),
                    ));
                }

                let content = read_body(body, *stdin)?;
                validation::validate_body(&content)?;
                let label_vec: Vec<String> = labels
                    .as_deref()
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                validation::validate_labels(&label_vec)?;

                let page = repo::create_page(
                    &conn,
                    &space_id,
                    parent.as_deref(),
                    title,
                    page_type,
                    &content,
                    sections_value.as_ref(),
                    &label_vec,
                    &identity.user,
                    &identity.agent,
                )?;
                output::print(mode, &page, || output::print_pretty_page(&page));
            }
            PageAction::Get { id } => {
                let page = repo::get_page(&conn, id)?;
                output::print(mode, &page, || output::print_pretty_page(&page));
            }
            PageAction::Update {
                id,
                title,
                body,
                stdin,
                sections,
                version,
                labels,
            } => {
                if let Some(ref t) = title {
                    validation::validate_title(t)?;
                }
                let content = if *stdin {
                    Some(read_body(&None, true)?)
                } else {
                    body.clone()
                };
                if let Some(ref c) = content {
                    validation::validate_body(c)?;
                }

                // Parse and validate sections JSON if provided
                if let Some(ref s) = sections {
                    validation::validate_sections_json(s)?;
                }
                let sections_value: Option<serde_json::Value> = match sections {
                    Some(ref s) => {
                        let val: serde_json::Value = serde_json::from_str(s)
                            .map_err(|e| db::KbError::InvalidInput(format!("Invalid sections JSON: {}", e)))?;
                        Some(val)
                    }
                    None => None,
                };

                repo::update_page(
                    &conn,
                    id,
                    title.as_deref(),
                    content.as_deref(),
                    sections_value.as_ref(),
                    *version,
                )?;

                if let Some(ref l) = labels {
                    let label_vec: Vec<String> = l
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    validation::validate_labels(&label_vec)?;
                    repo::set_labels(&conn, id, &label_vec)?;
                }

                let page = repo::get_page(&conn, id)?;
                output::print(mode, &page, || output::print_pretty_page(&page));
            }
            PageAction::Append { id, body, stdin } => {
                let content = read_body(body, *stdin)?;
                validation::validate_body(&content)?;
                if content.is_empty() {
                    return Err(db::KbError::InvalidInput(
                        "No content to append. Use --body or --stdin.".to_string(),
                    ));
                }
                let page = repo::append_to_page(&conn, id, &content)?;
                output::print(mode, &page, || output::print_pretty_page(&page));
            }
            PageAction::List {
                space,
                r#type,
                label,
                created_by_user,
                created_by_agent,
            } => {
                let space_id = match space {
                    Some(slug) => Some(resolve_space_id(&conn, slug)?),
                    None => None,
                };
                let page_type = match r#type {
                    Some(t) => Some(parse_page_type(t)?),
                    None => None,
                };
                let filters = repo::PageFilters {
                    space_id,
                    page_type,
                    label: label.clone(),
                    created_by_user: created_by_user.clone(),
                    created_by_agent: created_by_agent.clone(),
                };
                let pages = repo::list_pages(&conn, &filters)?;
                output::print(mode, &pages, || output::print_pretty_pages(&pages));
            }
            PageAction::Delete { id } => {
                repo::delete_page(&conn, id)?;
                let msg = serde_json::json!({"deleted": id});
                output::print(mode, &msg, || println!("Deleted page '{}'", id));
            }
            PageAction::Schema { r#type } => {
                let page_type = parse_page_type(r#type)?;
                match page_type.section_schema() {
                    Some(schema) => {
                        output::print(mode, &schema, || {
                            println!("Sections for '{}' pages:", page_type);
                            println!();
                            for def in &schema {
                                let req = if def.required { " (required)" } else { "" };
                                println!("  {}: {}{}", def.key, def.name, req);
                            }
                        });
                    }
                    None => {
                        let msg = serde_json::json!({
                            "page_type": page_type.as_str(),
                            "sections": null,
                            "note": "This page type uses freeform content (no structured sections)"
                        });
                        output::print(mode, &msg, || {
                            println!("Page type '{}' uses freeform content (no structured sections).", page_type);
                        });
                    }
                }
            }
        },

        // =====================================================================
        // Search command
        // =====================================================================
        Commands::Search {
            query,
            space,
            r#type,
            label,
            created_by_agent,
            section,
        } => {
            if let Some(ref q) = query {
                validation::validate_body(q)?; // reuse body limit as upper bound
            }
            let space_id = match space {
                Some(slug) => Some(resolve_space_id(&conn, slug)?),
                None => None,
            };
            let page_type = match r#type {
                Some(t) => Some(parse_page_type(t)?),
                None => None,
            };
            let params = search::SearchParams {
                query: query.clone(),
                space_id,
                page_type,
                label: label.clone(),
                created_by_agent: created_by_agent.clone(),
                section: section.clone(),
            };
            let results = search::search_pages(&conn, &params)?;
            output::print(mode, &results, || {
                output::print_pretty_search_results(&results)
            });
        }

        // =====================================================================
        // Link commands
        // =====================================================================
        Commands::Link { action } => match action {
            LinkAction::Create {
                source,
                target,
                relation,
            } => {
                let rel = parse_link_relation(relation)?;
                let link = repo::create_link(&conn, source, target, rel)?;
                output::print(mode, &link, || output::print_pretty_link(&link));
            }
            LinkAction::List { page_id } => {
                let links = repo::list_links(&conn, page_id)?;
                output::print(mode, &links, || output::print_pretty_links(&links));
            }
            LinkAction::Delete { source, target } => {
                repo::delete_link(&conn, source, target)?;
                let msg = serde_json::json!({"deleted": {"source": source, "target": target}});
                output::print(mode, &msg, || {
                    println!("Deleted link {} -> {}", source, target)
                });
            }
        },

        // =====================================================================
        // Browse command (interactive TUI)
        // =====================================================================
        Commands::Browse => {
            tui::run_browse(&conn)?;
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        // Output errors as JSON for agent consumption.
        let error_json = serde_json::json!({
            "error": e.to_string()
        });
        eprintln!("{}", serde_json::to_string(&error_json).unwrap());
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::validation;

    // ===== Slug validation =====

    #[test]
    fn test_slug_valid() {
        assert!(validation::validate_slug("my-project").is_ok());
        assert!(validation::validate_slug("project123").is_ok());
        assert!(validation::validate_slug("a").is_ok());
        assert!(validation::validate_slug("test_space").is_ok());
    }

    #[test]
    fn test_slug_rejects_empty() {
        assert!(validation::validate_slug("").is_err());
    }

    #[test]
    fn test_slug_rejects_uppercase() {
        assert!(validation::validate_slug("MyProject").is_err());
    }

    #[test]
    fn test_slug_rejects_spaces() {
        assert!(validation::validate_slug("my project").is_err());
    }

    #[test]
    fn test_slug_rejects_special_chars() {
        assert!(validation::validate_slug("my$project").is_err());
    }

    #[test]
    fn test_slug_must_start_with_letter() {
        assert!(validation::validate_slug("123abc").is_err());
        assert!(validation::validate_slug("-abc").is_err());
    }

    #[test]
    fn test_slug_rejects_too_long() {
        assert!(validation::validate_slug(&"a".repeat(129)).is_err());
    }

    #[test]
    fn test_slug_at_max_length() {
        assert!(validation::validate_slug(&"a".repeat(128)).is_ok());
    }

    // ===== Title validation =====

    #[test]
    fn test_title_valid() {
        assert!(validation::validate_title("My Decision").is_ok());
    }

    #[test]
    fn test_title_rejects_empty() {
        assert!(validation::validate_title("").is_err());
    }

    #[test]
    fn test_title_rejects_too_long() {
        assert!(validation::validate_title(&"x".repeat(501)).is_err());
    }

    // ===== Body validation =====

    #[test]
    fn test_body_at_limit() {
        assert!(validation::validate_body(&"x".repeat(10_000_000)).is_ok());
    }

    #[test]
    fn test_body_over_limit() {
        assert!(validation::validate_body(&"x".repeat(10_000_001)).is_err());
    }

    // ===== Label validation =====

    #[test]
    fn test_labels_valid() {
        let labels: Vec<String> = vec!["rust".into(), "security".into()];
        assert!(validation::validate_labels(&labels).is_ok());
    }

    #[test]
    fn test_labels_too_many() {
        let labels: Vec<String> = (0..51).map(|i| format!("l{}", i)).collect();
        assert!(validation::validate_labels(&labels).is_err());
    }

    #[test]
    fn test_label_too_long() {
        let labels = vec!["x".repeat(101)];
        assert!(validation::validate_labels(&labels).is_err());
    }

    // ===== Name validation =====

    #[test]
    fn test_name_valid() {
        assert!(validation::validate_name("My Project").is_ok());
    }

    #[test]
    fn test_name_at_limit() {
        assert!(validation::validate_name(&"x".repeat(256)).is_ok());
    }

    #[test]
    fn test_name_over_limit() {
        assert!(validation::validate_name(&"x".repeat(257)).is_err());
    }

    // ===== Description validation =====

    #[test]
    fn test_description_valid() {
        assert!(validation::validate_description("A description").is_ok());
    }

    #[test]
    fn test_description_at_limit() {
        assert!(validation::validate_description(&"x".repeat(2000)).is_ok());
    }

    #[test]
    fn test_description_over_limit() {
        assert!(validation::validate_description(&"x".repeat(2001)).is_err());
    }

    // ===== Sections JSON validation =====

    #[test]
    fn test_sections_json_valid() {
        assert!(validation::validate_sections_json("{}").is_ok());
    }

    #[test]
    fn test_sections_json_at_limit() {
        assert!(validation::validate_sections_json(&"x".repeat(10_000_000)).is_ok());
    }

    #[test]
    fn test_sections_json_over_limit() {
        assert!(validation::validate_sections_json(&"x".repeat(10_000_001)).is_err());
    }

    // ===== Additional boundary tests =====

    #[test]
    fn test_body_empty_is_valid() {
        assert!(validation::validate_body("").is_ok());
    }

    #[test]
    fn test_labels_empty_is_valid() {
        let labels: Vec<String> = vec![];
        assert!(validation::validate_labels(&labels).is_ok());
    }

    #[test]
    fn test_label_at_boundary() {
        let labels: Vec<String> = vec!["x".repeat(100)];
        assert!(validation::validate_labels(&labels).is_ok());
    }

    #[test]
    fn test_title_at_limit() {
        assert!(validation::validate_title(&"x".repeat(500)).is_ok());
    }
}
