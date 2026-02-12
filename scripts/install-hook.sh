#!/usr/bin/env bash
#
# Installs the whatidid session-summary hook for Claude Code.
#
# Usage:
#   ./scripts/install-hook.sh           # Global install (all projects)
#   ./scripts/install-hook.sh --project  # Project-level only (current repo)
#
# What it does:
#   1. Checks that dependencies are available (jq, whatidid, curl)
#   2. Copies the hook script to ~/.local/share/whatidid/hooks/
#   3. Merges the hook config into ~/.claude/settings.json
#
# Pass --uninstall to reverse the process.

set -euo pipefail

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOK_SRC="$REPO_DIR/hooks/session-summary.sh"

GLOBAL_HOOK_DIR="$HOME/.local/share/whatidid/hooks"
GLOBAL_HOOK_DST="$GLOBAL_HOOK_DIR/session-summary.sh"
GLOBAL_SETTINGS="$HOME/.claude/settings.json"

PROJECT_SETTINGS="$REPO_DIR/.claude/settings.json"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m  ✓\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m  !\033[0m %s\n' "$*"; }
error() { printf '\033[1;31m  ✗\033[0m %s\n' "$*" >&2; }

# Check if a JSON file has the SessionEnd hook already configured
has_hook() {
  local file="$1"
  [[ -f "$file" ]] && jq -e '.hooks.SessionEnd // empty | length > 0' "$file" >/dev/null 2>&1
}

# Merge the SessionEnd hook into an existing settings file, preserving all other keys
merge_hook_config() {
  local file="$1"
  local hook_command="$2"

  local hook_entry
  hook_entry="$(jq -n --arg cmd "$hook_command" '{
    hooks: {
      SessionEnd: [{
        hooks: [{
          type: "command",
          command: $cmd,
          timeout: 60
        }]
      }]
    }
  }')"

  if [[ -f "$file" ]]; then
    local existing
    existing="$(cat "$file")"

    if has_hook "$file"; then
      warn "SessionEnd hook already present in $file — skipping"
      return 0
    fi

    # Deep merge: existing settings + hook config
    printf '%s' "$existing" | jq --argjson hook "$hook_entry" '
      . * $hook
    ' > "${file}.tmp" && mv "${file}.tmp" "$file"
  else
    mkdir -p "$(dirname "$file")"
    printf '%s\n' "$hook_entry" | jq '.' > "$file"
  fi
}

# Remove the SessionEnd hook from a settings file, preserving everything else
remove_hook_config() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    return 0
  fi

  if ! has_hook "$file"; then
    warn "No SessionEnd hook found in $file — nothing to remove"
    return 0
  fi

  jq 'del(.hooks.SessionEnd) |
    if .hooks == {} then del(.hooks) else . end' "$file" > "${file}.tmp" \
    && mv "${file}.tmp" "$file"

  # Remove the file if it's now empty ({})
  if jq -e '. == {}' "$file" >/dev/null 2>&1; then
    rm "$file"
    ok "Removed empty $file"
  fi
}

# ---------------------------------------------------------------------------
# Dependency check
# ---------------------------------------------------------------------------
check_deps() {
  local missing=()
  for cmd in jq curl; do
    if ! command -v "$cmd" &>/dev/null; then
      missing+=("$cmd")
    fi
  done

  if ! command -v whatidid &>/dev/null; then
    warn "whatidid not found on PATH — install it with: cargo install --path $REPO_DIR"
  fi

  if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
    warn "ANTHROPIC_API_KEY is not set — the hook needs it at runtime"
  fi

  if [[ ${#missing[@]} -gt 0 ]]; then
    error "Missing required tools: ${missing[*]}"
    error "Install them and try again."
    exit 1
  fi
}

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------
ensure_session_log_space() {
  if ! command -v whatidid &>/dev/null; then
    warn "whatidid not on PATH — skipping session-log space creation"
    return 0
  fi

  if whatidid space list 2>/dev/null \
      | jq -e '.[] | select(.slug == "session-log")' >/dev/null 2>&1; then
    ok "session-log space already exists"
  else
    if whatidid space create session-log \
        --name "Session Log" \
        --description "Auto-generated session summaries from Claude Code hooks" 2>/dev/null; then
      ok "Created session-log space"
    else
      warn "Failed to create session-log space (will be created on first hook run)"
    fi
  fi
}

install_global() {
  info "Installing hook globally (all Claude Code sessions)"

  check_deps

  if [[ ! -f "$HOOK_SRC" ]]; then
    error "Hook script not found at $HOOK_SRC"
    exit 1
  fi

  # Copy hook script
  mkdir -p "$GLOBAL_HOOK_DIR"
  cp "$HOOK_SRC" "$GLOBAL_HOOK_DST"
  chmod +x "$GLOBAL_HOOK_DST"
  ok "Copied hook to $GLOBAL_HOOK_DST"

  # Merge into global settings
  merge_hook_config "$GLOBAL_SETTINGS" "$GLOBAL_HOOK_DST"
  ok "Hook registered in $GLOBAL_SETTINGS"

  # Create the session-log space so it's ready immediately
  ensure_session_log_space

  echo ""
  info "Done! The hook will run at the end of every Claude Code session."
  info "Session summaries will be written to whatidid, organized by project."
}

install_project() {
  info "Installing hook for this project only"

  check_deps

  if [[ ! -f "$HOOK_SRC" ]]; then
    error "Hook script not found at $HOOK_SRC"
    exit 1
  fi

  chmod +x "$HOOK_SRC"

  merge_hook_config "$PROJECT_SETTINGS" "\$CLAUDE_PROJECT_DIR/hooks/session-summary.sh"
  ok "Hook registered in $PROJECT_SETTINGS"

  # Create the session-log space so it's ready immediately
  ensure_session_log_space

  echo ""
  info "Done! The hook will run at the end of Claude Code sessions in this project."
}

# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------
uninstall() {
  info "Uninstalling session-summary hook"

  # Remove global hook
  if [[ -f "$GLOBAL_HOOK_DST" ]]; then
    rm "$GLOBAL_HOOK_DST"
    ok "Removed $GLOBAL_HOOK_DST"
    rmdir "$GLOBAL_HOOK_DIR" 2>/dev/null || true
  fi

  remove_hook_config "$GLOBAL_SETTINGS"
  ok "Cleaned $GLOBAL_SETTINGS"

  remove_hook_config "$PROJECT_SETTINGS"
  ok "Cleaned $PROJECT_SETTINGS"

  echo ""
  info "Done! Hook has been uninstalled."
}

# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
MODE="global"
for arg in "$@"; do
  case "$arg" in
    --project)   MODE="project" ;;
    --global)    MODE="global" ;;
    --uninstall) MODE="uninstall" ;;
    --help|-h)
      echo "Usage: $0 [--global | --project | --uninstall]"
      echo ""
      echo "  --global     Install for all Claude Code sessions (default)"
      echo "  --project    Install for this project only"
      echo "  --uninstall  Remove the hook from both global and project settings"
      exit 0
      ;;
    *)
      error "Unknown option: $arg"
      echo "Run $0 --help for usage."
      exit 1
      ;;
  esac
done

case "$MODE" in
  global)    install_global ;;
  project)   install_project ;;
  uninstall) uninstall ;;
esac
