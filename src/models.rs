//! Core data structures for the knowledge base.
//!
//! These structs are the shared language between the repository layer (SQL),
//! the CLI layer (clap), and the output layer (serde_json). They are kept
//! simple — plain data, no business logic.

use serde::Serialize;
use serde_json;

/// A top-level organizational unit. Not tied to a git repo — can represent
/// any project, team, or domain the user wants to organize knowledge around.
#[derive(Debug, Clone, Serialize)]
pub struct Space {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

/// The primary knowledge document. Belongs to a space, has a type that hints
/// at its structure, and optionally nests under a parent page for hierarchy.
#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub id: String,
    pub space_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub page_type: PageType,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sections: Option<serde_json::Value>,
    pub created_by_user: String,
    pub created_by_agent: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
    /// Labels attached to this page (populated on read, not stored in the pages table).
    pub labels: Vec<String>,
}

/// Constrained set of page types. Each suggests a different content structure,
/// but the content itself is freeform markdown — no hard schema enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageType {
    Decision,
    Architecture,
    SessionLog,
    Reference,
    Troubleshooting,
    Runbook,
}

impl PageType {
    /// Parse from a CLI string. Returns None for unrecognized types.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "decision" => Some(Self::Decision),
            "architecture" => Some(Self::Architecture),
            "session-log" => Some(Self::SessionLog),
            "reference" => Some(Self::Reference),
            "troubleshooting" => Some(Self::Troubleshooting),
            "runbook" => Some(Self::Runbook),
            _ => None,
        }
    }

    /// The string stored in SQLite and displayed in output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Architecture => "architecture",
            Self::SessionLog => "session-log",
            Self::Reference => "reference",
            Self::Troubleshooting => "troubleshooting",
            Self::Runbook => "runbook",
        }
    }

    /// Returns the expected section schema for this page type, if any.
    /// Returns None for freeform types (SessionLog, Reference).
    pub fn section_schema(&self) -> Option<Vec<SectionDef>> {
        match self {
            Self::Decision => Some(vec![
                SectionDef { key: "context", name: "Context", required: true },
                SectionDef { key: "options_considered", name: "Options Considered", required: true },
                SectionDef { key: "decision", name: "Decision", required: true },
                SectionDef { key: "consequences", name: "Consequences", required: false },
            ]),
            Self::Troubleshooting => Some(vec![
                SectionDef { key: "problem", name: "Problem", required: true },
                SectionDef { key: "diagnosis", name: "Diagnosis", required: true },
                SectionDef { key: "solution", name: "Solution", required: true },
            ]),
            Self::Architecture => Some(vec![
                SectionDef { key: "context", name: "Context", required: true },
                SectionDef { key: "design", name: "Design", required: true },
                SectionDef { key: "rationale", name: "Rationale", required: false },
                SectionDef { key: "constraints", name: "Constraints", required: false },
            ]),
            Self::Runbook => Some(vec![
                SectionDef { key: "prerequisites", name: "Prerequisites", required: false },
                SectionDef { key: "steps", name: "Steps", required: true },
                SectionDef { key: "rollback", name: "Rollback", required: false },
            ]),
            Self::SessionLog | Self::Reference => None,
        }
    }
}

impl std::fmt::Display for PageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Definition of a section within a page type's schema.
#[derive(Debug, Clone, Serialize)]
pub struct SectionDef {
    /// The key used in the sections JSON object.
    pub key: &'static str,
    /// Human-readable display name for the section.
    pub name: &'static str,
    /// Whether this section is required for the page type.
    pub required: bool,
}

/// Convert structured sections JSON to flat markdown text.
/// Iterates in schema-defined order for typed pages, or alphabetical for unknown types.
pub fn sections_to_content(sections: &serde_json::Value, page_type: PageType) -> String {
    let obj = match sections.as_object() {
        Some(o) => o,
        None => return String::new(),
    };

    let mut parts = Vec::new();

    if let Some(schema) = page_type.section_schema() {
        // Render in schema-defined order
        for def in &schema {
            if let Some(val) = obj.get(def.key) {
                if let Some(text) = val.as_str() {
                    parts.push(format!("## {}\n{}", def.name, text));
                }
            }
        }
        // Also include any extra keys not in the schema
        for (key, val) in obj {
            if !schema.iter().any(|d| d.key == key) {
                if let Some(text) = val.as_str() {
                    let title = key.replace('_', " ");
                    let title = title.split_whitespace()
                        .map(|w| {
                            let mut c = w.chars();
                            match c.next() {
                                None => String::new(),
                                Some(f) => f.to_uppercase().to_string() + c.as_str(),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    parts.push(format!("## {}\n{}", title, text));
                }
            }
        }
    } else {
        // Freeform type: alphabetical order
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(text) = obj[key].as_str() {
                let title = key.replace('_', " ");
                let title = title.split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().to_string() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                parts.push(format!("## {}\n{}", title, text));
            }
        }
    }

    parts.join("\n\n")
}

/// A typed directional relationship between two pages.
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub source_id: String,
    pub target_id: String,
    pub relation: LinkRelation,
    pub created_at: String,
    pub updated_at: String,
}

/// The kind of relationship between two linked pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkRelation {
    RelatesTo,
    Supersedes,
    DependsOn,
    Elaborates,
}

impl LinkRelation {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "relates-to" => Some(Self::RelatesTo),
            "supersedes" => Some(Self::Supersedes),
            "depends-on" => Some(Self::DependsOn),
            "elaborates" => Some(Self::Elaborates),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RelatesTo => "relates-to",
            Self::Supersedes => "supersedes",
            Self::DependsOn => "depends-on",
            Self::Elaborates => "elaborates",
        }
    }
}

impl std::fmt::Display for LinkRelation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A search result with a relevance snippet from FTS5.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub page: Page,
    /// FTS5 snippet showing the matched text in context. Empty for non-FTS queries.
    pub excerpt: String,
}

/// Identity of the agent performing an operation.
/// Resolved from CLI flags, env vars, or system defaults.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub user: String,
    pub agent: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_type_roundtrip() {
        let types = [
            "decision",
            "architecture",
            "session-log",
            "reference",
            "troubleshooting",
            "runbook",
        ];
        for t in types {
            let parsed = PageType::from_str(t).unwrap_or_else(|| panic!("should parse '{}'", t));
            assert_eq!(parsed.as_str(), t);
        }
    }

    #[test]
    fn page_type_rejects_unknown() {
        assert!(PageType::from_str("blog-post").is_none());
    }

    #[test]
    fn link_relation_roundtrip() {
        let relations = ["relates-to", "supersedes", "depends-on", "elaborates"];
        for r in relations {
            let parsed =
                LinkRelation::from_str(r).unwrap_or_else(|| panic!("should parse '{}'", r));
            assert_eq!(parsed.as_str(), r);
        }
    }

    #[test]
    fn link_relation_rejects_unknown() {
        assert!(LinkRelation::from_str("blocks").is_none());
    }

    #[test]
    fn decision_has_section_schema() {
        let schema = PageType::Decision.section_schema().expect("Decision should have schema");
        assert_eq!(schema.len(), 4);
        assert_eq!(schema[0].key, "context");
        assert!(schema[0].required);
        assert_eq!(schema[3].key, "consequences");
        assert!(!schema[3].required);
    }

    #[test]
    fn session_log_has_no_schema() {
        assert!(PageType::SessionLog.section_schema().is_none());
    }

    #[test]
    fn reference_has_no_schema() {
        assert!(PageType::Reference.section_schema().is_none());
    }

    #[test]
    fn sections_to_content_decision() {
        let sections = serde_json::json!({
            "context": "We needed a database.",
            "options_considered": "1. Postgres\n2. SQLite",
            "decision": "SQLite for simplicity.",
            "consequences": "No network dependency."
        });
        let content = sections_to_content(&sections, PageType::Decision);
        assert!(content.contains("## Context\nWe needed a database."));
        assert!(content.contains("## Decision\nSQLite for simplicity."));
        // Verify order: Context should come before Decision
        let ctx_pos = content.find("## Context").unwrap();
        let dec_pos = content.find("## Decision").unwrap();
        assert!(ctx_pos < dec_pos);
    }

    #[test]
    fn sections_to_content_empty_object() {
        let sections = serde_json::json!({});
        let content = sections_to_content(&sections, PageType::Decision);
        assert_eq!(content, "");
    }

    #[test]
    fn sections_to_content_non_object() {
        let sections = serde_json::json!("not an object");
        let content = sections_to_content(&sections, PageType::Decision);
        assert_eq!(content, "");
    }

    #[test]
    fn test_sections_to_content_freeform_type() {
        let sections = serde_json::json!({
            "beta": "Second entry.",
            "alpha": "First entry."
        });
        let content = sections_to_content(&sections, PageType::SessionLog);
        let alpha_pos = content.find("## Alpha").unwrap();
        let beta_pos = content.find("## Beta").unwrap();
        assert!(alpha_pos < beta_pos);
    }

    #[test]
    fn test_sections_to_content_extra_keys_on_typed() {
        let sections = serde_json::json!({
            "context": "Some context.",
            "custom_field": "Extra info."
        });
        let content = sections_to_content(&sections, PageType::Decision);
        assert!(content.contains("## Context\nSome context."));
        assert!(content.contains("## Custom Field\nExtra info."));
        // Schema keys come before extra keys
        let ctx_pos = content.find("## Context").unwrap();
        let custom_pos = content.find("## Custom Field").unwrap();
        assert!(ctx_pos < custom_pos);
    }

    #[test]
    fn test_sections_to_content_non_string_values() {
        let sections = serde_json::json!({
            "context": 42
        });
        let content = sections_to_content(&sections, PageType::Decision);
        assert_eq!(content, "");
    }

    #[test]
    fn test_page_type_display() {
        assert_eq!(format!("{}", PageType::Decision), "decision");
        assert_eq!(format!("{}", PageType::Architecture), "architecture");
        assert_eq!(format!("{}", PageType::SessionLog), "session-log");
        assert_eq!(format!("{}", PageType::Reference), "reference");
        assert_eq!(format!("{}", PageType::Troubleshooting), "troubleshooting");
        assert_eq!(format!("{}", PageType::Runbook), "runbook");
    }

    #[test]
    fn test_link_relation_display() {
        assert_eq!(format!("{}", LinkRelation::RelatesTo), "relates-to");
        assert_eq!(format!("{}", LinkRelation::Supersedes), "supersedes");
        assert_eq!(format!("{}", LinkRelation::DependsOn), "depends-on");
        assert_eq!(format!("{}", LinkRelation::Elaborates), "elaborates");
    }

    #[test]
    fn test_architecture_section_schema() {
        let schema = PageType::Architecture.section_schema().expect("Architecture should have schema");
        assert_eq!(schema.len(), 4);
        assert_eq!(schema[0].key, "context");
    }

    #[test]
    fn test_troubleshooting_section_schema() {
        let schema = PageType::Troubleshooting.section_schema().expect("Troubleshooting should have schema");
        assert_eq!(schema.len(), 3);
        assert_eq!(schema[0].key, "problem");
        assert_eq!(schema[1].key, "diagnosis");
        assert_eq!(schema[2].key, "solution");
    }

    #[test]
    fn test_runbook_section_schema() {
        let schema = PageType::Runbook.section_schema().expect("Runbook should have schema");
        assert_eq!(schema.len(), 3);
        assert_eq!(schema[0].key, "prerequisites");
        assert_eq!(schema[1].key, "steps");
        assert_eq!(schema[2].key, "rollback");
    }
}
