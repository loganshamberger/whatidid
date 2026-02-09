# Knowledge Base (kb) — Architecture & Design Decisions

## What This Is

A local CLI knowledge management tool for AI agents. Think "Confluence for agents" — a
structured place where AI agents working on a project can write, read, and search knowledge
documents. Runs entirely offline, no internet required.

## Core Design Decisions

### Storage: SQLite (single file)

- **Why**: Agents don't browse filesystems for knowledge — they run commands. SQLite gives us
  structured queries, ACID guarantees, and concurrent access in a single portable file.
- **WAL mode**: Enabled on every connection for concurrent reads with serialized writes.
  Handles multi-agent access without explicit locking.
- **FTS5**: Used for full-text search over page titles and content.
- **Location**: `~/.knowledge-base/kb.db` by default, overridden via `KB_PATH` env var.

### Language: Rust

- Memory safe (compile-time guarantees).
- Single static binary — no runtime dependencies on the target machine.
- Fast startup — matters when agents invoke the CLI many times per session.
- Key crates: `clap` (CLI), `rusqlite` (SQLite with bundled FTS5), `serde`/`serde_json`
  (serialization), `uuid`, `chrono`.

### CLI-First Interface

- Every command outputs JSON by default (agents are the primary consumer).
- `--pretty` flag for human-readable output.
- `--stdin` flag on write operations so agents can pipe markdown content without shell escaping.
- Designed to be invoked as a subprocess by any agent framework.

### Agent Identity

Two-dimensional identity: **human user** + **agent tool**.

- `created_by_user`: The human who initiated the session (e.g., "logan").
- `created_by_agent`: The tool doing the writing (e.g., "claude-code").
- Population order: explicit `--user`/`--agent` flags > `KB_USER`/`KB_AGENT` env vars >
  `$USER` for the human (agent has no default — tools must identify themselves).

### Migrations

Schema versioning from day one. `schema_meta` table tracks the current version. Migration
files in `migrations/` are numbered sequentially (`001_initial.sql`, `002_whatever.sql`).
Every CLI invocation checks the schema version and runs pending migrations automatically
in a transaction. Failed migrations roll back cleanly.

### Optimistic Concurrency

Pages have a `version` integer that increments on every update. Updates can pass
`--version N` to ensure they're modifying the expected version. If the version has changed
(another agent wrote in between), the update fails with a clear error rather than silently
overwriting.

## Data Model

### Entities

- **Space**: Top-level organizational unit. User-defined, not tied to a git repo.
  Has a human-friendly slug used in CLI commands.
- **Page**: The primary knowledge document. Has a type, belongs to a space, optionally
  has a parent (for hierarchy). Content is markdown.
- **Label**: Tags on pages for categorization. Many-to-many.
- **Link**: Explicit typed relationships between pages.

### Page Types

| Type              | Purpose                          | Suggested Sections                                      |
|-------------------|----------------------------------|---------------------------------------------------------|
| `decision`        | Record a choice and rationale    | Context, Options Considered, Decision, Consequences     |
| `architecture`    | System/component design          | Overview, Components, Interfaces, Constraints           |
| `session-log`     | Running log of an agent session  | Append-only timestamped entries                         |
| `reference`       | Stable factual information       | Content-dependent                                       |
| `troubleshooting` | Problems and solutions           | Problem, Diagnosis, Solution                            |
| `runbook`         | Step-by-step procedures          | Prerequisites, Steps, Verification                      |

### Link Relations

- `relates-to` (default)
- `supersedes`
- `depends-on`
- `elaborates`

## CLI Contract

```
kb space create <slug> [--name "..."] [--description "..."]
kb space list
kb space get <slug>
kb space delete <slug>

kb page create --space <slug> --title "..." --type <type> [--parent <id>] [--labels a,b,c] [--body "..." | --stdin]
kb page get <id>
kb page update <id> [--title "..."] [--body "..." | --stdin] [--version <n>]
kb page append <id> --body "..."
kb page list [--space <slug>] [--type <type>] [--label <label>] [--created-by-user <user>] [--created-by-agent <agent>]
kb page delete <id>

kb search <query> [--space <slug>] [--type <type>] [--label <label>] [--created-by-agent <agent>]

kb link create <source-id> <target-id> [--relation relates-to]
kb link list <page-id>
kb link delete <source-id> <target-id>
```

Global flags: `--pretty` (human-readable output), `--user`, `--agent`.

## Project Structure

```
src/
├── main.rs      -- clap CLI entry point, dispatches to commands
├── db.rs        -- connection management, WAL setup, migration runner
├── models.rs    -- data structs (Space, Page, Label, Link, SearchResult)
├── repo.rs      -- all SQL queries (CRUD for every entity)
├── search.rs    -- FTS5 query builder with structured filters
└── output.rs    -- JSON serialization + --pretty formatting

migrations/
└── 001_initial.sql

tests/
└── integration.rs
```

## Search

Full-text search via FTS5, combined with structured filters (space, type, label, agent)
in a single query. FTS index is kept in sync via SQLite triggers. When no search text is
given, falls back to metadata-only filtering.

## Future Considerations (Not In Scope Yet)

- Version history / page diffs
- MCP server interface for direct agent tool integration
- Export to markdown files
- Multi-machine sync
- Access control / permissions
