#!/usr/bin/env bash
# Convenience script: search whatidid and format results as markdown context.
#
# Usage: scripts/search-context.sh "search terms"
#
# This is a standalone version of the search logic from the
# search-before-solving hook, useful for manual testing and debugging.

set -euo pipefail

if ! command -v whatidid &>/dev/null; then
    echo "Error: whatidid is not installed. Run: cargo install --path ." >&2
    exit 1
fi

if [ $# -eq 0 ] || [ -z "$1" ]; then
    echo "Usage: $0 \"search terms\"" >&2
    exit 1
fi

query="$1"

echo "## Searching whatidid for: $query"
echo ""

# General search
echo "### General Results"
general="$(whatidid search "$query" 2>/dev/null || true)"
echo "$general" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    if not data:
        print('(no results)')
    else:
        for item in data[:10]:
            page = item.get('page', item)
            title = page.get('title', 'Untitled')
            ptype = page.get('page_type', '')
            pid = page.get('id', '')[:8]
            excerpt = item.get('excerpt', '')
            labels = ', '.join(page.get('labels', []))
            print(f'- **{title}** [{ptype}] (id: {pid}...) {\"labels: \" + labels if labels else \"\"}')
            if excerpt:
                print(f'  > {excerpt[:300]}')
except:
    print('(no results or parse error)')
" 2>/dev/null
echo ""

# Pitfall search
echo "### Known Pitfalls"
pitfalls="$(whatidid search "$query" --label pitfall 2>/dev/null || true)"
echo "$pitfalls" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    if not data:
        print('(no pitfalls found)')
    else:
        for item in data[:5]:
            page = item.get('page', item)
            title = page.get('title', 'Untitled')
            ptype = page.get('page_type', '')
            pid = page.get('id', '')[:8]
            excerpt = item.get('excerpt', '')
            print(f'- **{title}** [{ptype}] (id: {pid}...)')
            if excerpt:
                print(f'  > {excerpt[:300]}')
except:
    print('(no pitfalls or parse error)')
" 2>/dev/null
