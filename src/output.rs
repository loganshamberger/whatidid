//! Output formatting for the knowledge base CLI.
//!
//! This module provides two output modes:
//! - **JSON**: Compact machine-readable output (default for agent consumers)
//! - **Pretty**: Human-readable formatted output (enabled via `--pretty` flag)
//!
//! The JSON format uses serde_json to serialize models directly, ensuring
//! stable machine-readable output. The pretty format emphasizes readability
//! with labeled fields and structured layouts.

use crate::models::{Link, Page, SearchResult, Space};
use serde::Serialize;

/// Output mode for CLI results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Compact JSON output (default for agents).
    Json,
    /// Human-readable formatted output.
    Pretty,
}

/// Serialize a value to compact JSON and print to stdout.
///
/// This is the default output mode for agents. The JSON is compact (no pretty-printing)
/// to minimize token usage when agents parse the output.
///
/// # Panics
///
/// Panics if serialization fails, which should only happen if the type has a broken
/// `Serialize` implementation.
pub fn print_json<T: Serialize>(value: &T) {
    let json = serde_json::to_string(value)
        .expect("failed to serialize to JSON");
    println!("{}", json);
}

/// Print a space in human-readable format.
///
/// Format:
/// ```text
/// Space: my-project
/// Name:  My Project
/// ID:    <uuid>
/// Desc:  Some description
/// ```
pub fn print_pretty_space(space: &Space) {
    println!("Space: {}", space.slug);
    println!("Name:  {}", space.name);
    println!("ID:    {}", space.id);
    println!("Desc:  {}", space.description);
    println!("Created: {}", space.created_at);
    println!("Updated: {}", space.updated_at);
}

/// Print a page in human-readable format.
///
/// Format:
/// ```text
/// Title:   Some Decision
/// ID:      <uuid>
/// Space:   <space_id>
/// Type:    decision
/// Labels:  security, auth
/// Author:  logan / claude-code
/// Version: 3
/// Created: 2024-01-15T10:30:00Z
/// Updated: 2024-01-15T11:00:00Z
///
/// <content>
/// ```
pub fn print_pretty_page(page: &Page) {
    println!("Title:   {}", page.title);
    println!("ID:      {}", page.id);
    println!("Space:   {}", page.space_id);
    println!("Type:    {}", page.page_type);

    if page.labels.is_empty() {
        println!("Labels:  (none)");
    } else {
        println!("Labels:  {}", page.labels.join(", "));
    }

    println!("Author:  {} / {}", page.created_by_user, page.created_by_agent);
    println!("Version: {}", page.version);
    println!("Created: {}", page.created_at);
    println!("Updated: {}", page.updated_at);
    println!();
    if let Some(ref sections) = page.sections {
        if let Some(obj) = sections.as_object() {
            // Try to get schema-defined order
            let ordered_keys: Vec<(&str, &str)> = if let Some(schema) = page.page_type.section_schema() {
                schema.iter().map(|d| (d.key, d.name)).collect()
            } else {
                // Freeform: alphabetical
                let mut keys: Vec<&String> = obj.keys().collect();
                keys.sort();
                keys.into_iter().map(|k| (k.as_str(), k.as_str())).collect()
            };

            let mut first = true;
            for (key, display_name) in &ordered_keys {
                if let Some(val) = obj.get(*key) {
                    if let Some(text) = val.as_str() {
                        if !first {
                            println!();
                        }
                        first = false;
                        println!("--- {} ---", display_name);
                        println!("{}", text);
                    }
                }
            }
            // Any extra keys not in schema
            if let Some(schema) = page.page_type.section_schema() {
                for (key, val) in obj {
                    if !schema.iter().any(|d| d.key == key) {
                        if let Some(text) = val.as_str() {
                            if !first {
                                println!();
                            }
                            first = false;
                            println!("--- {} ---", key);
                            println!("{}", text);
                        }
                    }
                }
            }
        } else {
            println!("{}", page.content);
        }
    } else {
        println!("{}", page.content);
    }
}

/// Print a list of pages as a table-like summary.
///
/// Each page is printed on one line with key fields: id, type, title, author.
/// This provides a quick overview when listing multiple pages.
///
/// Format: `<id> | <type> | <title> | <user>/<agent>`
pub fn print_pretty_pages(pages: &[Page]) {
    if pages.is_empty() {
        println!("(no pages)");
        return;
    }

    for page in pages {
        println!(
            "{} | {} | {} | {}/{}",
            page.id,
            page.page_type,
            page.title,
            page.created_by_user,
            page.created_by_agent
        );
    }
}

/// Print search results with excerpts.
///
/// Each result shows the page's key metadata and the FTS5 excerpt showing
/// matched text in context. If no excerpt is available, falls back to
/// showing the first 200 characters of content.
pub fn print_pretty_search_results(results: &[SearchResult]) {
    if results.is_empty() {
        println!("(no results)");
        return;
    }

    for (i, result) in results.iter().enumerate() {
        if i > 0 {
            println!();
        }

        println!("Title:  {}", result.page.title);
        println!("ID:     {}", result.page.id);
        println!("Type:   {}", result.page.page_type);
        println!("Space:  {}", result.page.space_id);

        if !result.page.labels.is_empty() {
            println!("Labels: {}", result.page.labels.join(", "));
        }

        if !result.excerpt.is_empty() {
            println!("Match:  {}", result.excerpt);
        } else {
            // Fallback to showing content prefix if no excerpt
            let preview = if result.page.content.len() > 200 {
                // Find the largest char boundary <= 200 to avoid panicking on multibyte UTF-8.
                let mut end = 200;
                while end > 0 && !result.page.content.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &result.page.content[..end])
            } else {
                result.page.content.clone()
            };
            println!("Preview: {}", preview);
        }
    }
}

/// Print a single link in human-readable format.
///
/// Format: `<source_id> --[<relation>]--> <target_id> (<created_at>)`
pub fn print_pretty_link(link: &Link) {
    println!("{} --[{}]--> {} ({})", link.source_id, link.relation, link.target_id, link.created_at);
}

/// Print a list of links.
///
/// Each link is printed on one line showing the relationship graphically.
pub fn print_pretty_links(links: &[Link]) {
    if links.is_empty() {
        println!("(no links)");
        return;
    }

    for link in links {
        print_pretty_link(link);
    }
}

/// Generic output dispatcher that handles both JSON and Pretty modes.
///
/// This helper function chooses between JSON serialization and a custom
/// pretty-print function based on the output mode.
///
/// # Arguments
///
/// * `mode` - The output mode (Json or Pretty)
/// * `value` - The value to output (must be serializable for JSON mode)
/// * `pretty_fn` - A closure that prints the value in pretty format
///
/// # Example
///
/// ```ignore
/// let space = Space { ... };
/// print(mode, &space, || print_pretty_space(&space));
/// ```
pub fn print<T: Serialize>(mode: OutputMode, value: &T, pretty_fn: impl FnOnce()) {
    match mode {
        OutputMode::Json => print_json(value),
        OutputMode::Pretty => pretty_fn(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Link, LinkRelation, Page, PageType, SearchResult, Space};

    // Test fixtures

    fn fixture_space() -> Space {
        Space {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            slug: "test-project".to_string(),
            name: "Test Project".to_string(),
            description: "A test space for unit tests".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            updated_at: "2024-01-15T10:00:00Z".to_string(),
        }
    }

    fn fixture_page() -> Page {
        Page {
            id: "660e8400-e29b-41d4-a716-446655440001".to_string(),
            space_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            parent_id: None,
            title: "Test Decision".to_string(),
            page_type: PageType::Decision,
            content: "# Test Decision\n\nThis is a test decision page.".to_string(),
            sections: None,
            created_by_user: "testuser".to_string(),
            created_by_agent: "test-agent".to_string(),
            created_at: "2024-01-15T11:00:00Z".to_string(),
            updated_at: "2024-01-15T12:00:00Z".to_string(),
            version: 1,
            labels: vec!["test".to_string(), "decision".to_string()],
        }
    }

    fn fixture_page_no_labels() -> Page {
        Page {
            labels: vec![],
            ..fixture_page()
        }
    }

    fn fixture_page_with_sections() -> Page {
        Page {
            sections: Some(serde_json::json!({
                "context": "We needed a database.",
                "decision": "Use SQLite.",
                "options_considered": "1. Postgres\n2. SQLite",
                "consequences": "Single-file storage."
            })),
            ..fixture_page()
        }
    }

    fn fixture_search_result() -> SearchResult {
        SearchResult {
            page: fixture_page(),
            excerpt: "This is a test <b>decision</b> page.".to_string(),
        }
    }

    fn fixture_search_result_no_excerpt() -> SearchResult {
        SearchResult {
            page: fixture_page(),
            excerpt: "".to_string(),
        }
    }

    fn fixture_link() -> Link {
        Link {
            source_id: "660e8400-e29b-41d4-a716-446655440001".to_string(),
            target_id: "770e8400-e29b-41d4-a716-446655440002".to_string(),
            relation: LinkRelation::RelatesTo,
            created_at: "2024-01-15T13:00:00Z".to_string(),
            updated_at: "2024-01-15T13:00:00Z".to_string(),
        }
    }

    // JSON output tests - verify that JSON is valid and parseable

    #[test]
    fn test_space_json_is_valid() {
        let space = fixture_space();
        let json = serde_json::to_string(&space).expect("should serialize");

        // Verify we can parse it back
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        // Verify expected fields are present
        assert_eq!(parsed["slug"], "test-project");
        assert_eq!(parsed["name"], "Test Project");
        assert_eq!(parsed["id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(parsed["description"], "A test space for unit tests");
        assert_eq!(parsed["created_at"], "2024-01-15T10:00:00Z");
        assert_eq!(parsed["updated_at"], "2024-01-15T10:00:00Z");
    }

    #[test]
    fn test_page_json_is_valid() {
        let page = fixture_page();
        let json = serde_json::to_string(&page).expect("should serialize");

        // Verify we can parse it back
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        // Verify expected fields are present
        assert_eq!(parsed["title"], "Test Decision");
        assert_eq!(parsed["id"], "660e8400-e29b-41d4-a716-446655440001");
        assert_eq!(parsed["space_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(parsed["page_type"], "decision");
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["labels"][0], "test");
        assert_eq!(parsed["labels"][1], "decision");
        assert_eq!(parsed["created_by_user"], "testuser");
        assert_eq!(parsed["created_by_agent"], "test-agent");
    }

    #[test]
    fn test_page_with_no_labels_json_is_valid() {
        let page = fixture_page_no_labels();
        let json = serde_json::to_string(&page).expect("should serialize");

        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        // Verify labels is an empty array
        assert!(parsed["labels"].is_array());
        assert_eq!(parsed["labels"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_search_result_json_is_valid() {
        let result = fixture_search_result();
        let json = serde_json::to_string(&result).expect("should serialize");

        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        // Verify structure
        assert!(parsed["page"].is_object());
        assert_eq!(parsed["excerpt"], "This is a test <b>decision</b> page.");
        assert_eq!(parsed["page"]["title"], "Test Decision");
    }

    #[test]
    fn test_link_json_is_valid() {
        let link = fixture_link();
        let json = serde_json::to_string(&link).expect("should serialize");

        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert_eq!(parsed["source_id"], "660e8400-e29b-41d4-a716-446655440001");
        assert_eq!(parsed["target_id"], "770e8400-e29b-41d4-a716-446655440002");
        assert_eq!(parsed["relation"], "relates-to");
        assert_eq!(parsed["created_at"], "2024-01-15T13:00:00Z");
        assert_eq!(parsed["updated_at"], "2024-01-15T13:00:00Z");
    }

    #[test]
    fn test_multiple_pages_json_is_valid() {
        let pages = vec![fixture_page(), fixture_page_no_labels()];
        let json = serde_json::to_string(&pages).expect("should serialize");

        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_multiple_search_results_json_is_valid() {
        let results = vec![
            fixture_search_result(),
            fixture_search_result_no_excerpt(),
        ];
        let json = serde_json::to_string(&results).expect("should serialize");

        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["excerpt"], "This is a test <b>decision</b> page.");
        assert_eq!(parsed[1]["excerpt"], "");
    }

    #[test]
    fn test_multiple_links_json_is_valid() {
        let links = vec![
            fixture_link(),
            Link {
                source_id: "770e8400-e29b-41d4-a716-446655440002".to_string(),
                target_id: "880e8400-e29b-41d4-a716-446655440003".to_string(),
                relation: LinkRelation::Supersedes,
                created_at: "2024-01-15T13:00:00Z".to_string(),
                updated_at: "2024-01-15T13:00:00Z".to_string(),
            },
        ];
        let json = serde_json::to_string(&links).expect("should serialize");

        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["relation"], "relates-to");
        assert_eq!(parsed[1]["relation"], "supersedes");
    }

    // Tests for all PageType variants serializing correctly

    #[test]
    fn test_all_page_types_serialize_correctly() {
        let types = vec![
            (PageType::Decision, "decision"),
            (PageType::Architecture, "architecture"),
            (PageType::SessionLog, "session-log"),
            (PageType::Reference, "reference"),
            (PageType::Troubleshooting, "troubleshooting"),
            (PageType::Runbook, "runbook"),
        ];

        for (page_type, expected_json_value) in types {
            let page = Page {
                page_type,
                ..fixture_page()
            };
            let json = serde_json::to_string(&page).expect("should serialize");
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .expect("should parse as valid JSON");

            assert_eq!(parsed["page_type"], expected_json_value);
        }
    }

    // Tests for all LinkRelation variants serializing correctly

    #[test]
    fn test_all_link_relations_serialize_correctly() {
        let relations = vec![
            (LinkRelation::RelatesTo, "relates-to"),
            (LinkRelation::Supersedes, "supersedes"),
            (LinkRelation::DependsOn, "depends-on"),
            (LinkRelation::Elaborates, "elaborates"),
        ];

        for (relation, expected_json_value) in relations {
            let mut link = fixture_link();
            link.relation = relation;
            let json = serde_json::to_string(&link).expect("should serialize");
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .expect("should parse as valid JSON");

            assert_eq!(parsed["relation"], expected_json_value);
        }
    }

    // Test OutputMode enum

    #[test]
    fn test_output_mode_equality() {
        assert_eq!(OutputMode::Json, OutputMode::Json);
        assert_eq!(OutputMode::Pretty, OutputMode::Pretty);
        assert_ne!(OutputMode::Json, OutputMode::Pretty);
    }

    // Test print function dispatcher
    // Note: We can't easily test the actual printed output in unit tests without
    // capturing stdout, but we can verify the function compiles and runs without panic.

    #[test]
    fn test_print_json_mode_does_not_panic() {
        let space = fixture_space();
        // This test verifies the function doesn't panic - output goes to stdout
        // which we don't capture in this unit test
        print(OutputMode::Json, &space, || {
            panic!("pretty_fn should not be called in JSON mode");
        });
    }

    #[test]
    fn test_print_pretty_mode_calls_pretty_fn() {
        let space = fixture_space();
        let mut called = false;

        // In Pretty mode, the closure should be called, not JSON serialization
        print(OutputMode::Pretty, &space, || {
            called = true;
        });

        assert!(called, "pretty_fn should be called in Pretty mode");
    }

    // Edge case tests

    #[test]
    fn test_page_with_long_content_serializes() {
        let long_content = "x".repeat(10000);
        let page = Page {
            content: long_content.clone(),
            ..fixture_page()
        };

        let json = serde_json::to_string(&page).expect("should serialize long content");
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert_eq!(parsed["content"].as_str().unwrap().len(), 10000);
    }

    #[test]
    fn test_page_with_special_characters_serializes() {
        let page = Page {
            title: "Test \"quotes\" and 'apostrophes'".to_string(),
            content: "Content with\nnewlines\tand\ttabs".to_string(),
            ..fixture_page()
        };

        let json = serde_json::to_string(&page).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert!(parsed["title"].as_str().unwrap().contains("quotes"));
        assert!(parsed["content"].as_str().unwrap().contains("newlines"));
    }

    #[test]
    fn test_empty_collections_serialize() {
        let pages: Vec<Page> = vec![];
        let json = serde_json::to_string(&pages).expect("should serialize empty vec");
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_optional_parent_id_serializes_as_null() {
        let page = Page {
            parent_id: None,
            ..fixture_page()
        };

        let json = serde_json::to_string(&page).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert!(parsed["parent_id"].is_null());
    }

    #[test]
    fn test_optional_parent_id_with_value_serializes() {
        let page = Page {
            parent_id: Some("parent-uuid-123".to_string()),
            ..fixture_page()
        };

        let json = serde_json::to_string(&page).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("should parse as valid JSON");

        assert_eq!(parsed["parent_id"], "parent-uuid-123");
    }

    #[test]
    fn test_page_with_sections_json_is_valid() {
        let page = fixture_page_with_sections();
        let json = serde_json::to_string(&page).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should parse");
        assert!(parsed["sections"].is_object());
        assert_eq!(parsed["sections"]["context"], "We needed a database.");
    }

    // ===== Security: Unicode panic tests =====

    #[test]
    fn test_pretty_search_results_cjk_content_does_not_panic() {
        // 67 CJK chars = 201 bytes; byte 200 falls inside the 67th char
        let content = "\u{4E2D}".repeat(67);
        assert!(content.len() > 200);
        assert!(!content.is_char_boundary(200));
        let results = vec![SearchResult {
            page: Page { content, ..fixture_page() },
            excerpt: "".to_string(),
        }];
        print_pretty_search_results(&results); // must not panic
    }

    #[test]
    fn test_pretty_search_results_emoji_content_does_not_panic() {
        // 51 emoji = 204 bytes; byte 200 falls inside the 51st emoji
        let content = "\u{1F600}".repeat(51);
        assert!(content.len() > 200);
        let results = vec![SearchResult {
            page: Page { content, ..fixture_page() },
            excerpt: "".to_string(),
        }];
        print_pretty_search_results(&results); // must not panic
    }

    #[test]
    fn test_page_without_sections_json_omits_field() {
        let page = fixture_page();
        let json = serde_json::to_string(&page).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should parse");
        // sections should be absent (not null) due to skip_serializing_if
        assert!(parsed.get("sections").is_none());
    }

    // ===== Pretty-print smoke tests =====

    #[test]
    fn test_print_pretty_space_does_not_panic() {
        print_pretty_space(&fixture_space());
    }

    #[test]
    fn test_print_pretty_page_does_not_panic() {
        print_pretty_page(&fixture_page());
    }

    #[test]
    fn test_print_pretty_page_no_labels_does_not_panic() {
        print_pretty_page(&fixture_page_no_labels());
    }

    #[test]
    fn test_print_pretty_page_with_sections_does_not_panic() {
        print_pretty_page(&fixture_page_with_sections());
    }

    #[test]
    fn test_print_pretty_pages_empty() {
        print_pretty_pages(&[]);
    }

    #[test]
    fn test_print_pretty_pages_multiple() {
        let pages = vec![fixture_page(), fixture_page_no_labels()];
        print_pretty_pages(&pages);
    }

    #[test]
    fn test_print_pretty_links_empty() {
        print_pretty_links(&[]);
    }

    #[test]
    fn test_print_pretty_links_multiple() {
        let links = vec![
            fixture_link(),
            Link {
                source_id: "770e8400-e29b-41d4-a716-446655440002".to_string(),
                target_id: "880e8400-e29b-41d4-a716-446655440003".to_string(),
                relation: LinkRelation::Supersedes,
                created_at: "2024-01-15T14:00:00Z".to_string(),
                updated_at: "2024-01-15T14:00:00Z".to_string(),
            },
        ];
        print_pretty_links(&links);
    }

    #[test]
    fn test_print_pretty_search_results_empty() {
        print_pretty_search_results(&[]);
    }

    #[test]
    fn test_print_pretty_search_results_with_excerpt() {
        let results = vec![fixture_search_result()];
        print_pretty_search_results(&results);
    }

    #[test]
    fn test_print_pretty_search_results_no_excerpt_short_content() {
        let results = vec![fixture_search_result_no_excerpt()];
        print_pretty_search_results(&results);
    }

    #[test]
    fn test_print_pretty_link_does_not_panic() {
        print_pretty_link(&fixture_link());
    }

    #[test]
    fn test_page_with_empty_labels_serializes() {
        let page = fixture_page_no_labels();
        let json = serde_json::to_string(&page).expect("should serialize");
        assert!(json.contains("\"labels\":[]"));
    }
}
