# whatidid

A local CLI knowledge base for AI agents. Stores structured knowledge documents (pages) organized into spaces, backed by SQLite with FTS5 full-text search. All output is JSON by default for agent consumption, with `--pretty` for humans.

### This is in active development. Not for production use.

## Install

```bash
cargo install --path .
```

Requires `~/.cargo/bin` on your `PATH`. Verify with:

```bash
whatidid --version
```

## Quick Start

```bash
# Create a space
whatidid space create my-project --name "My Project" --description "Project knowledge base"

# Add a decision page
whatidid page create --space my-project --title "Chose PostgreSQL over MySQL" \
  --type decision --labels "database,architecture" \
  --body "We chose PostgreSQL for its JSON support and extensibility."

# Add a structured decision page (with typed sections)
whatidid page create --space my-project --title "REST over GraphQL" \
  --type decision --labels "api-design,tradeoff" \
  --sections '{"context":"Need a public API","options_considered":"1. REST\n2. GraphQL","decision":"REST for simplicity","consequences":"Less flexible for frontend"}'

# Search across all spaces
whatidid search "database"

# Browse interactively
whatidid browse
```

## Global Flags

| Flag | Description |
|------|-------------|
| `--pretty` | Human-readable output instead of JSON |
| `--user <NAME>` | Override user identity (default: `$KB_USER`, `$USER`, or "unknown") |
| `--agent <NAME>` | Override agent identity (default: `$KB_AGENT` or "unknown") |

## Commands

### `space` -- Manage spaces

Spaces are top-level organizational units (e.g., one per project).

```bash
whatidid space create <SLUG> [--name <NAME>] [--description <TEXT>]
whatidid space list
whatidid space get <SLUG>
whatidid space delete <SLUG>    # Must have no pages
```

### `page` -- Manage pages

Pages are knowledge documents within a space.

```bash
# Create
whatidid page create --space <SLUG> --title <TITLE> --type <TYPE> \
  [--body <TEXT> | --stdin | --sections <JSON>] \
  [--labels <LABEL1,LABEL2>] [--parent <PAGE_ID>]

# Read
whatidid page get <ID>
whatidid page list [--space <SLUG>] [--type <TYPE>] [--label <LABEL>] \
  [--created-by-user <USER>] [--created-by-agent <AGENT>]

# Update
whatidid page update <ID> [--title <TITLE>] [--body <TEXT> | --stdin | --sections <JSON>] \
  [--version <N>]

# Append content
whatidid page append <ID> --body <TEXT>
whatidid page append <ID> --stdin

# Delete
whatidid page delete <ID>

# Show section schema for a page type
whatidid page schema --type <TYPE>
```

### `search` -- Full-text search

```bash
whatidid search [QUERY] [--space <SLUG>] [--type <TYPE>] [--label <LABEL>] \
  [--created-by-agent <AGENT>] [--section <KEY>]
```

All filters are AND'd together. Without a query, only metadata filters apply. With a query, results are ranked by FTS5 relevance and include text excerpts.

### `link` -- Manage relationships between pages

```bash
whatidid link create <SOURCE_ID> <TARGET_ID> [--relation <RELATION>]
whatidid link list <PAGE_ID>
whatidid link delete <SOURCE_ID> <TARGET_ID>
```

**Link relations**: `relates-to` (default), `supersedes`, `depends-on`, `elaborates`

### `browse` -- Interactive TUI

Launches a terminal UI for browsing spaces and pages with vim-like navigation:

| Key | Action |
|-----|--------|
| `j`/`k` or arrows | Navigate up/down |
| `Enter` | Select / drill into |
| `h`/`Esc` | Go back |
| `l`/`Tab` | Focus content pane |
| `/` | Search |
| `e` | Edit page in $EDITOR |
| `gg` / `G` | Jump to top / bottom |
| `q` | Quit |

## Page Types

| Type | Sections |
|------|----------|
| **decision** | `context` (req), `options_considered` (req), `decision` (req), `consequences` (opt) |
| **troubleshooting** | `problem` (req), `diagnosis` (req), `solution` (req) |
| **architecture** | `context` (req), `design` (req), `rationale` (opt), `constraints` (opt) |
| **runbook** | `prerequisites` (opt), `steps` (req), `rollback` (opt) |
| **session-log** | Freeform (no schema) |
| **reference** | Freeform (no schema) |

Use `whatidid page schema --type <TYPE>` to see expected sections. Pages with sections have their `content` auto-derived for full-text indexing. Use `--body` for freeform content or `--sections` for structured content (mutually exclusive).

## Optimistic Concurrency

Pages have a `version` field (starts at 1, incremented on each update). Pass `--version` on update to detect concurrent modifications:

```bash
whatidid page update <ID> --title "Updated" --version 1
# Fails with VersionConflict if current version != 1
```

## Claude Code Hook

The included `hooks/session-summary.sh` hook automatically summarizes every Claude Code session and writes it to whatidid as a `session-log` page. On session end it:

1. Detects changed files via `git diff --stat`
2. Extracts the conversation transcript
3. Calls the Anthropic API (Haiku) to produce a structured summary
4. Writes a page to whatidid with **Summary**, **Open Questions**, and **Files Changed**

### Requirements

- `whatidid` on `PATH`
- `jq` on `PATH`
- `ANTHROPIC_API_KEY` environment variable set

The hook fails silently if any dependency is missing — it will never break your session.

### Install

Use the install script:

```bash
# Global — all Claude Code sessions, all projects (default)
./scripts/install-hook.sh

# Project-only — just this repo
./scripts/install-hook.sh --project

# Uninstall — removes from both global and project settings
./scripts/install-hook.sh --uninstall
```

The global install copies the hook to `~/.local/share/whatidid/hooks/` and registers it in `~/.claude/settings.json`. The project install uses the hook in-place from the repo via `.claude/settings.json`.

The hook auto-creates a whatidid space per project directory (derived from the folder name), so session logs are organized by project without any manual setup.

## Configuration

| Setting | Default | Override |
|---------|---------|----------|
| Database path | `~/.knowledge-base/kb.db` | `KB_PATH` env var |
| User identity | `$USER` or "unknown" | `--user` flag or `KB_USER` env var |
| Agent identity | "unknown" | `--agent` flag or `KB_AGENT` env var |

The database directory is created automatically on first run. SQLite runs in WAL mode with foreign keys enabled.

## Development

```bash
cargo build                # Debug build
cargo build --release      # Release build
cargo test                 # Run all ~105 unit tests
cargo test -- test_name    # Run a single test
cargo clippy               # Lint
```

## License

[MIT](LICENSE)
