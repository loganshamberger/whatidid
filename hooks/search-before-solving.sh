#!/usr/bin/env bash
# Hook: UserPromptSubmit — search whatidid for prior knowledge before solving.
#
# Reads the user prompt from stdin JSON, extracts keywords, searches the
# knowledge base, and returns matching results as context for the agent.
#
# Exits silently (0, no output) when:
#   - whatidid is not installed
#   - The prompt is too short (< 3 meaningful words)
#   - No search results are found

set -euo pipefail

# Ensure whatidid is available
if ! command -v whatidid &>/dev/null; then
    exit 0
fi

# Read the hook input from stdin
input="$(cat)"

# Extract the user prompt text from the JSON
prompt="$(echo "$input" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(data.get('prompt', ''))
except:
    print('')
" 2>/dev/null)"

if [ -z "$prompt" ]; then
    exit 0
fi

# Stop words to exclude from keyword extraction
stop_words="the and for are but not you all any can had her was one our out day get has him his how its may new now old see way who did about after being could every from have into just like make more most need only should some than that them then these they this very what when where which while will with would your"

# Extract meaningful keywords (> 4 chars, not stop words)
all_keywords=()
for word in $prompt; do
    # Strip punctuation and lowercase
    clean="$(echo "$word" | tr -d '[:punct:]' | tr '[:upper:]' '[:lower:]')"
    if [ ${#clean} -le 4 ]; then
        continue
    fi
    # Check against stop words
    if echo " $stop_words " | grep -q " $clean "; then
        continue
    fi
    all_keywords+=("$clean")
done

# Skip if fewer than 3 meaningful keywords
if [ "${#all_keywords[@]}" -lt 3 ]; then
    exit 0
fi

# Use only the top 5 keywords to keep FTS queries focused
# (FTS5 quoted strings match as exact phrases, so shorter = better)
keywords_for_search=("${all_keywords[@]:0:5}")

# Run separate searches per keyword to maximize hits
# (since FTS5 quotes the full string as a phrase, single-word queries work best)
general_results="[]"
pitfall_results="[]"
for kw in "${keywords_for_search[@]}"; do
    result="$(whatidid search "$kw" 2>/dev/null || echo '[]')"
    general_results="$(python3 -c "
import json, sys
existing = json.loads('''$general_results''')
new = json.loads(sys.stdin.read())
seen_ids = {item.get('page', item).get('id') for item in existing}
for item in new:
    pid = item.get('page', item).get('id')
    if pid not in seen_ids:
        existing.append(item)
        seen_ids.add(pid)
print(json.dumps(existing[:10]))
" <<< "$result" 2>/dev/null || echo "$general_results")"

    pitfall_result="$(whatidid search "$kw" --label pitfall 2>/dev/null || echo '[]')"
    pitfall_results="$(python3 -c "
import json, sys
existing = json.loads('''$pitfall_results''')
new = json.loads(sys.stdin.read())
seen_ids = {item.get('page', item).get('id') for item in existing}
for item in new:
    pid = item.get('page', item).get('id')
    if pid not in seen_ids:
        existing.append(item)
        seen_ids.add(pid)
print(json.dumps(existing[:5]))
" <<< "$pitfall_result" 2>/dev/null || echo "$pitfall_results")"
done

# Check if we got any results
general_count="$(echo "$general_results" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(len(data) if isinstance(data, list) else 0)
except:
    print(0)
" 2>/dev/null)"

pitfall_count="$(echo "$pitfall_results" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(len(data) if isinstance(data, list) else 0)
except:
    print(0)
" 2>/dev/null)"

if [ "$general_count" = "0" ] && [ "$pitfall_count" = "0" ]; then
    exit 0
fi

# Format results as context
context=""

if [ "$pitfall_count" != "0" ]; then
    pitfall_text="$(echo "$pitfall_results" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for item in data[:3]:
    page = item.get('page', item)
    title = page.get('title', 'Untitled')
    ptype = page.get('page_type', '')
    excerpt = item.get('excerpt', '')
    labels = ', '.join(page.get('labels', []))
    print(f'- **{title}** [{ptype}] {\"(\" + labels + \")\" if labels else \"\"}')
    if excerpt:
        print(f'  > {excerpt[:200]}')
" 2>/dev/null)"
    context="${context}### Known Pitfalls\n${pitfall_text}\n\n"
fi

if [ "$general_count" != "0" ]; then
    general_text="$(echo "$general_results" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for item in data[:5]:
    page = item.get('page', item)
    title = page.get('title', 'Untitled')
    ptype = page.get('page_type', '')
    excerpt = item.get('excerpt', '')
    labels = ', '.join(page.get('labels', []))
    print(f'- **{title}** [{ptype}] {\"(\" + labels + \")\" if labels else \"\"}')
    if excerpt:
        print(f'  > {excerpt[:200]}')
" 2>/dev/null)"
    context="${context}### Related Prior Knowledge\n${general_text}\n"
fi

# Output the context as a JSON message for the hook system
python3 -c "
import json, sys
context = '''## Prior Knowledge from whatidid

$context
Review these results and reference any relevant findings in your approach.'''
print(json.dumps({'result': context}))
"
