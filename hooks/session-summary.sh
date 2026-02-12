#!/usr/bin/env bash
#
# Claude Code SessionEnd hook — summarizes the session and writes it to whatidid.
#
# Triggers on: SessionEnd
# Type: command (receives JSON on stdin)
#
# Requirements:
#   - whatidid on PATH (cargo install --path . from this repo)
#   - jq on PATH
#   - ANTHROPIC_API_KEY environment variable set
#   - Transcript JSONL from Claude Code (passed via hook input)
#
# Installation:
#   Add to .claude/settings.json (project) or ~/.claude/settings.json (global):
#
#   {
#     "hooks": {
#       "SessionEnd": [{
#         "hooks": [{
#           "type": "command",
#           "command": "$CLAUDE_PROJECT_DIR/hooks/session-summary.sh"
#         }]
#       }]
#     }
#   }

set -euo pipefail

# ---------------------------------------------------------------------------
# Read hook input from stdin
# ---------------------------------------------------------------------------
INPUT="$(cat)"

TRANSCRIPT_PATH="$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty')"
SESSION_ID="$(printf '%s' "$INPUT" | jq -r '.session_id // empty')"
CWD="$(printf '%s' "$INPUT" | jq -r '.cwd // empty')"

# Bail silently if there's no transcript to summarize
if [[ -z "$TRANSCRIPT_PATH" || ! -f "$TRANSCRIPT_PATH" ]]; then
  exit 0
fi

# ---------------------------------------------------------------------------
# Check dependencies
# ---------------------------------------------------------------------------
for cmd in jq whatidid curl; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "session-summary hook: '$cmd' not found in PATH, skipping" >&2
    exit 0
  fi
done

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  echo "session-summary hook: ANTHROPIC_API_KEY not set, skipping" >&2
  exit 0
fi

# ---------------------------------------------------------------------------
# Derive project label from directory name; write to a shared session-log space
# ---------------------------------------------------------------------------
PROJECT_DIR="${CWD:-$(pwd)}"
PROJECT_LABEL="$(basename "$PROJECT_DIR" \
  | tr '[:upper:]' '[:lower:]' \
  | sed 's/[^a-z0-9-]/-/g; s/--*/-/g; s/^-//; s/-$//')"

if [[ -z "$PROJECT_LABEL" ]]; then
  PROJECT_LABEL="misc"
fi

SPACE_SLUG="session-log"

# Ensure the session-log space exists (create if missing, ignore errors)
if ! whatidid space list 2>/dev/null \
    | jq -e ".[] | select(.slug == \"$SPACE_SLUG\")" >/dev/null 2>&1; then
  whatidid space create "$SPACE_SLUG" \
    --name "Session Log" \
    --description "Auto-generated session summaries from Claude Code hooks" 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# Extract modified files from transcript (Write, Edit, NotebookEdit tool calls)
# ---------------------------------------------------------------------------
# jq filter: from assistant messages, find tool_use blocks for file-modifying tools,
# extract the file path, deduplicate, and sort.
FILE_MODIFY_FILTER='
  select(.type == "assistant")
  | .message.content[]
  | select(.type == "tool_use" and (.name == "Write" or .name == "Edit" or .name == "NotebookEdit"))
  | (.input.file_path // .input.notebook_path)
'

# Collect from main transcript
MODIFIED_FILES="$(jq -r "$FILE_MODIFY_FILTER" "$TRANSCRIPT_PATH" 2>/dev/null || true)"

# Also collect from subagent transcripts (Task agents can edit files too).
# Subagent transcripts live in a sibling directory: <session-id>/subagents/*.jsonl
TRANSCRIPT_DIR="${TRANSCRIPT_PATH%.jsonl}"
if [[ -d "$TRANSCRIPT_DIR/subagents" ]]; then
  for subagent_file in "$TRANSCRIPT_DIR"/subagents/*.jsonl; do
    [[ -f "$subagent_file" ]] || continue
    SUB_FILES="$(jq -r "$FILE_MODIFY_FILTER" "$subagent_file" 2>/dev/null || true)"
    if [[ -n "$SUB_FILES" ]]; then
      MODIFIED_FILES="${MODIFIED_FILES}
${SUB_FILES}"
    fi
  done
fi

# Deduplicate and sort; strip empty lines
MODIFIED_FILES="$(printf '%s\n' "$MODIFIED_FILES" | grep -v '^$' | sort -u)" || true

# ---------------------------------------------------------------------------
# Extract git commits from transcript (Bash tool calls containing "git commit")
# ---------------------------------------------------------------------------
# Correlates tool_use IDs with their tool_result output to get the commit line.
# Uses jq -s (slurp) to join across JSONL lines.
GIT_COMMIT_FILTER='
  # Collect tool_use IDs for git commit bash calls
  [ .[]
    | select(.type == "assistant")
    | .message.content[]
    | select(.type == "tool_use" and .name == "Bash"
        and (.input.command | test("git commit")))
    | .id
  ] as $ids
  |
  # Find successful tool_results for those IDs, extract first line (hash + message).
  # Guard: user messages may have string content; only iterate arrays.
  [ .[]
    | select(.type == "user" and (.message.content | type) == "array")
    | .message.content[]
    | select(.type == "tool_result"
        and (.is_error | not)
        and (.tool_use_id as $tid | $ids | index($tid)))
    | .content
    | split("\n")[0]
    | select(startswith("["))
  ]
  | unique[]
'

GIT_COMMITS="$(jq -rs "$GIT_COMMIT_FILTER" "$TRANSCRIPT_PATH" 2>/dev/null || true)"

if [[ -d "$TRANSCRIPT_DIR/subagents" ]]; then
  for subagent_file in "$TRANSCRIPT_DIR"/subagents/*.jsonl; do
    [[ -f "$subagent_file" ]] || continue
    SUB_COMMITS="$(jq -rs "$GIT_COMMIT_FILTER" "$subagent_file" 2>/dev/null || true)"
    if [[ -n "$SUB_COMMITS" ]]; then
      GIT_COMMITS="${GIT_COMMITS}
${SUB_COMMITS}"
    fi
  done
fi

GIT_COMMITS="$(printf '%s\n' "$GIT_COMMITS" | grep -v '^$' | sort -u)" || true

# ---------------------------------------------------------------------------
# Extract whatidid decision pages created during the session
# ---------------------------------------------------------------------------
# Finds "whatidid page create --type decision" calls, parses the JSON result
# to get page ID and title.
DECISION_PAGE_FILTER='
  [ .[]
    | select(.type == "assistant")
    | .message.content[]
    | select(.type == "tool_use" and .name == "Bash"
        and (.input.command | test("whatidid page create.*--type decision")))
    | .id
  ] as $ids
  |
  # Guard: user messages may have string content; only iterate arrays.
  [ .[]
    | select(.type == "user" and (.message.content | type) == "array")
    | .message.content[]
    | select(.type == "tool_result"
        and (.is_error | not)
        and (.tool_use_id as $tid | $ids | index($tid)))
    | .content
    | fromjson?
    | select(.id and .title)
    | "- " + .title + " (id: " + .id + ")"
  ]
  | unique[]
'

DECISION_PAGES="$(jq -rs "$DECISION_PAGE_FILTER" "$TRANSCRIPT_PATH" 2>/dev/null || true)"

if [[ -d "$TRANSCRIPT_DIR/subagents" ]]; then
  for subagent_file in "$TRANSCRIPT_DIR"/subagents/*.jsonl; do
    [[ -f "$subagent_file" ]] || continue
    SUB_PAGES="$(jq -rs "$DECISION_PAGE_FILTER" "$subagent_file" 2>/dev/null || true)"
    if [[ -n "$SUB_PAGES" ]]; then
      DECISION_PAGES="${DECISION_PAGES}
${SUB_PAGES}"
    fi
  done
fi

DECISION_PAGES="$(printf '%s\n' "$DECISION_PAGES" | grep -v '^$' | sort -u)" || true

# ---------------------------------------------------------------------------
# Extract transcript content (condensed, last ~50KB)
# ---------------------------------------------------------------------------
# The transcript is JSONL. Pull human and assistant text messages.
TRANSCRIPT_CONTENT="$(jq -r '
  if .type == "user" then
    "USER: " + (
      if (.message.content | type) == "string" then .message.content
      elif (.message.content | type) == "array" then
        (.message.content | map(select(.type == "text") | .text) | join("\n"))
      else "(non-text input)"
      end
    )
  elif .type == "assistant" then
    "ASSISTANT: " + (
      if (.message.content | type) == "array" then
        (.message.content | map(select(.type == "text") | .text) | join("\n"))
      elif (.message.content | type) == "string" then .message.content
      else ""
      end
    )
  else empty
  end
' "$TRANSCRIPT_PATH" 2>/dev/null | tail -c 50000)" || true

if [[ -z "$TRANSCRIPT_CONTENT" ]]; then
  echo "session-summary hook: could not extract transcript, skipping" >&2
  exit 0
fi

# ---------------------------------------------------------------------------
# Build the summarization prompt
# ---------------------------------------------------------------------------
PROMPT="You are summarizing a Claude Code session for a developer knowledge base.
Analyze the transcript below, then produce a JSON object with exactly these keys:

- \"title\": A concise (5-10 word) title summarizing what was done this session.
- \"labels\": Comma-separated topic labels relevant to the work (e.g. \"rust,refactoring,bug-fix\"). Use lowercase kebab-case.
- \"summary\": 2-4 sentences describing what was accomplished.
- \"open_questions\": Bullet list (as a single string) of any unresolved questions, TODOs, or issues remaining. Use \"None\" if everything was resolved.

Respond with ONLY valid JSON. No markdown fencing, no commentary.

---

## Session Transcript (tail)
$TRANSCRIPT_CONTENT"

# ---------------------------------------------------------------------------
# Call Anthropic API (Haiku for speed + cost)
# ---------------------------------------------------------------------------
API_BODY="$(jq -n --arg prompt "$PROMPT" '{
  model: "claude-haiku-4-5-20251001",
  max_tokens: 1024,
  messages: [{role: "user", content: $prompt}]
}')"

RESPONSE="$(curl -s --max-time 30 https://api.anthropic.com/v1/messages \
  -H "content-type: application/json" \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  -d "$API_BODY" 2>/dev/null)" || true

# Extract the text from the API response
SUMMARY_RAW="$(printf '%s' "$RESPONSE" | jq -r '.content[0].text // empty' 2>/dev/null)" || true

if [[ -z "$SUMMARY_RAW" ]]; then
  echo "session-summary hook: API call failed or returned empty, skipping" >&2
  exit 0
fi

# Strip markdown fences (```json ... ```) that models sometimes add despite instructions
SUMMARY_JSON="$(printf '%s' "$SUMMARY_RAW" | sed 's/^```[a-z]*//; s/^```$//' | tr -d '\n' | sed 's/^ *//')"

# Validate that we got parseable JSON
if ! printf '%s' "$SUMMARY_JSON" | jq -e . >/dev/null 2>&1; then
  echo "session-summary hook: could not parse summary JSON, skipping" >&2
  exit 0
fi

# ---------------------------------------------------------------------------
# Parse the summary fields
# (handle both string and array formats since models are inconsistent)
# ---------------------------------------------------------------------------
TITLE="$(printf '%s' "$SUMMARY_JSON" | jq -r '.title // "Session Summary"' 2>/dev/null)" || TITLE="Session Summary"
LABELS="$(printf '%s' "$SUMMARY_JSON" | jq -r '
  if (.labels | type) == "array" then (.labels | join(","))
  else (.labels // "session")
  end
' 2>/dev/null)" || LABELS="session"
# Always include the project label so sessions are filterable by project
if [[ -n "$PROJECT_LABEL" ]] && ! echo "$LABELS" | grep -qw "$PROJECT_LABEL"; then
  LABELS="${LABELS},${PROJECT_LABEL}"
fi
SUMMARY="$(printf '%s' "$SUMMARY_JSON" | jq -r '.summary // "No summary available."' 2>/dev/null)" || SUMMARY="No summary available."
QUESTIONS="$(printf '%s' "$SUMMARY_JSON" | jq -r '
  if (.open_questions | type) == "array" then (.open_questions | map("- " + .) | join("\n"))
  else (.open_questions // "None")
  end
' 2>/dev/null)" || QUESTIONS="None"
FILES="${MODIFIED_FILES:-None}"
COMMITS="${GIT_COMMITS:-None}"
DECISIONS="${DECISION_PAGES:-None}"

# ---------------------------------------------------------------------------
# Write to whatidid
# ---------------------------------------------------------------------------
BODY="$(cat <<EOF
## Summary
$SUMMARY

## Open Questions
$QUESTIONS

## Files Changed
$FILES

## Git Commits
$COMMITS

## Decisions Recorded
$DECISIONS

---
*Project: $(basename "$PROJECT_DIR") | Session ID: $SESSION_ID*
EOF
)"

whatidid page create \
  --space "$SPACE_SLUG" \
  --title "$TITLE" \
  --type session-log \
  --agent claude-code \
  --labels "$LABELS" \
  --body "$BODY" 2>/dev/null || {
    echo "session-summary hook: failed to write to whatidid" >&2
    exit 0
  }

exit 0
