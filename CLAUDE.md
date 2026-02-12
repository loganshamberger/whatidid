# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`whatidid` is a local CLI knowledge management tool for AI agents. It stores structured knowledge documents (pages) organized into spaces, backed by SQLite with FTS5 full-text search. All output is JSON by default for agent consumption, with `--pretty` for humans. Pages support optional structured sections (JSON) alongside freeform content.

## Build, Test & Install

```bash
cargo build                    # Debug build (binary: target/debug/whatidid)
cargo build --release          # Release build (binary: target/release/whatidid)
cargo test                     # Run all tests (~105 unit tests across modules)
cargo test -- test_name        # Run a single test by name
cargo test repo::tests         # Run tests in a specific module
cargo clippy                   # Lint
```

### Installing

```bash
cargo install --path .         # Installs 'whatidid' to ~/.cargo/bin/
```

Requires `~/.cargo/bin` to be on `PATH`. Verify with `whatidid --version`.

## REQUIRED: Search Before Solving

**Before working on any task, agents MUST search whatidid for relevant prior knowledge.** Search across ALL projects (not just the current one) for pitfalls, patterns, and prior decisions that may inform the approach.

```bash
whatidid search "relevant keywords"                     # Cross-project full-text search
whatidid search "topic" --label pitfall                  # Find known pitfalls
whatidid search "topic" --label pattern                  # Find reusable patterns
whatidid page list --type decision --label sqlite        # All SQLite decisions across projects
```

If relevant results are found, reference them in your approach. This prevents repeating mistakes and leverages institutional knowledge.

> **Note**: A `UserPromptSubmit` hook (`hooks/search-before-solving.sh`) runs automatically on each prompt to surface relevant prior knowledge. If you see prior knowledge context injected, review it before proceeding.

## REQUIRED: Document Decisions with `whatidid`

**All Claude Code instances working in this repository MUST use `whatidid` to record decisions.**

> **Warning**: A Stop hook (`hooks/check-decisions.sh`) will block session completion if it detects discussed-but-unrecorded decisions. Record decisions as you go to avoid being blocked at the end.

The project space `knowledge-base` already exists. When you make a non-trivial decision (architecture choices, trade-offs, tool selections, approach changes, rejected alternatives), record it:

```bash
whatidid page create --space knowledge-base --title "Short decision title" --type decision --agent claude-code --labels "relevant,labels" --body "$(cat <<'EOF'
## Context
Why this decision was needed.

## Options Considered
1. Option A — description
2. Option B — description

## Decision
What was chosen and why.

## Consequences
Trade-offs and implications.
EOF
)"
```

Or using structured sections (preferred for typed pages):

```bash
whatidid page create --space knowledge-base --title "Short decision title" --type decision --agent claude-code --labels "relevant,labels" --sections '{"context":"Why this decision was needed.","options_considered":"1. Option A\n2. Option B","decision":"What was chosen and why.","consequences":"Trade-offs and implications."}'
```

### What counts as a "decision"

Record when a reasonable alternative existed. Examples:

- Choosing SQLite over Postgres for storage
- Using `INSERT OR IGNORE` vs `INSERT ... ON CONFLICT`
- Deciding to derive content from sections vs storing separately
- Picking an $EDITOR suspension pattern over an embedded editor for TUI editing
- Choosing to use external content FTS5 vs regular FTS5

**When NOT to record**: Trivial or forced choices (fixing a typo, following an explicit user instruction with no ambiguity).

### Label strategy

Labels make knowledge discoverable across projects. Apply at two levels:

- **Topic labels** — the technical domain: `sqlite`, `auth`, `caching`, `error-handling`, `testing`, `api-design`, `concurrency`, `rust`, `python`, `typescript`, etc.
- **Pattern labels** — the kind of lesson: `pitfall` (something that surprised you or broke), `pattern` (a reusable approach that worked well), `tradeoff` (a deliberate compromise worth remembering).

Always include at least one topic label. Add a pattern label when the decision contains a transferable lesson.

### Reviewing prior decisions

```bash
whatidid search "topic" --space knowledge-base          # Full-text search
whatidid search "topic" --section context               # Search pages with a specific section
whatidid page list --space knowledge-base --type decision  # List all decisions
whatidid page get <id>                                     # Read a specific decision
whatidid page schema --type decision                       # Show expected sections for a type
```

## RECOMMENDED: Document Project Structure

Agents should maintain a nested page hierarchy to document the project as they build:

```
Root: "Project Documentation" (type: reference, no parent)
├── L1: Module/component pages (type: architecture, parent = root)
│   ├── L2: Feature/topic pages (type: reference, parent = L1)
│   └── L2: Another feature page
├── L1: Another component
└── Troubleshooting pages (type: troubleshooting, linked via 'elaborates')
```

### Creating the hierarchy

```bash
# Create the root documentation page
whatidid page create --space knowledge-base --title "Project Documentation" \
  --type reference --agent claude-code --body "Top-level overview of the project."

# Create a module page (L1) — use the root page ID as parent
whatidid page create --space knowledge-base --title "TUI Module" \
  --type architecture --parent <root-page-id> --agent claude-code \
  --sections '{"context":"...","design":"...","rationale":"..."}'

# Create a feature page (L2) — use the module page ID as parent
whatidid page create --space knowledge-base --title "TUI Edit Mode" \
  --type reference --parent <module-page-id> --agent claude-code --body "..."

# Link a troubleshooting page to the module it relates to
whatidid link create <troubleshooting-page-id> <module-page-id> --relation elaborates
```

Agents should check for and maintain this hierarchy as they build. Use `whatidid page list --space knowledge-base` to see existing pages before creating new ones.

## Architecture

**Single-crate Rust binary** (`[[bin]] name = "whatidid"`) with six modules (no workspace, no lib crate):

- `main.rs` — clap CLI entry point. Parses args, dispatches to repo/search functions, formats output. All commands route through a single `run()` function.
- `db.rs` — SQLite connection setup (WAL mode, foreign keys, busy timeout) and migration runner. Defines `KbError`, the crate-wide error type.
- `models.rs` — Plain data structs (`Space`, `Page`, `Link`, `SearchResult`, `AgentIdentity`) plus `PageType`/`LinkRelation` enums. Also contains `SectionDef`, `PageType::section_schema()`, and `sections_to_content()`.
- `repo.rs` — All CRUD SQL queries as plain functions taking `&Connection`. Uses `row_to_page()` helper for consistent row mapping. Dynamic query building uses `Vec<Box<dyn ToSql>>` parameter lists (not `named_params!` macro, which rejects unused params).
- `search.rs` — FTS5 search combined with structured filters (including section-level filtering via `json_extract`). Builds SQL dynamically, binds only the parameters actually present.
- `output.rs` — JSON serialization (compact, for agents) and `--pretty` human-readable formatting via `print()` dispatcher.

**Data flow**: CLI args (clap) -> `run()` -> repo/search functions (rusqlite) -> output formatting -> stdout.

### Structured Sections

Pages can have optional structured sections (`sections` JSON column alongside `content`):

- `sections = NULL` → freeform page (legacy, SessionLog, Reference)
- `sections = '{"context":"...","decision":"..."}'` → structured page

Per-type section schemas:
- **Decision**: context (req), options_considered (req), decision (req), consequences (opt)
- **Troubleshooting**: problem (req), diagnosis (req), solution (req)
- **Architecture**: context (req), design (req), rationale (opt), constraints (opt)
- **Runbook**: prerequisites (opt), steps (req), rollback (opt)
- **SessionLog / Reference**: freeform (no schema)

When sections are provided, `content` is auto-derived via `sections_to_content()` for FTS indexing. Use `--sections` flag (mutually exclusive with `--body` on create). Use `page schema --type <type>` to discover expected sections.

## Key Technical Constraints

**FTS5 external content tables**: The FTS index uses `content='pages'` (external content). `snippet()` and `highlight()` do NOT work with this configuration in bundled SQLite. Excerpts are generated in Rust (`search.rs:make_excerpt()`), not via FTS5 auxiliary functions.

**FTS5 query safety**: Hyphens in search terms are interpreted as column filters by FTS5. User search terms must always be quoted: `"user input"` before passing to MATCH. Internal quotes are escaped via `q.replace('"', "\"\"")`.

**rusqlite named parameters**: The `named_params!{}` macro passes ALL params — rusqlite rejects params not present in the SQL. For dynamic queries (search, filtered lists), build `Vec<(&str, Box<dyn ToSql>)>` manually.

**Migrations**: Embedded via `include_str!("../migrations/NNN_*.sql")` in `db.rs`. Auto-run on every CLI invocation. Schema version tracked in `schema_meta` table. Currently at version 3 (001: initial, 002: sections, 003: timestamps).

**Optimistic concurrency**: Pages have a `version` integer. Updates with `--version N` fail with `VersionConflict` if the version has changed since the caller read it.

## Testing Patterns

All unit tests use **in-memory SQLite** with all three migration files applied directly (not through the migration runner). Each test creates its own database via a `setup_test_db()` helper function defined in each test module.

The `tests/` directory exists but is currently empty (no integration tests).

## Database

- **Location**: `~/.knowledge-base/kb.db` (override: `KB_PATH` env var)
- **Schema**: spaces, pages (with sections JSON column), labels (many-to-many), links (directed, typed), pages_fts (FTS5 virtual table), schema_meta
- **Timestamps**: All entities (spaces, pages, links) have `created_at` and `updated_at` fields
- **FTS sync**: Maintained via INSERT/UPDATE/DELETE triggers on the pages table
- **Edition**: `edition = "2021"` in Cargo.toml (not 2024, for broader crate compatibility)
