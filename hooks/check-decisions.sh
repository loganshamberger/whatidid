#!/usr/bin/env bash
# Hook: Stop — checks if unrecorded decisions exist in the conversation.
#
# Reads the conversation transcript, scans for decision-language keywords
# in assistant messages, and checks if corresponding `whatidid page create
# --type decision` commands were executed.
#
# Blocks (returns non-empty JSON) only when there is clear evidence of
# discussed but unrecorded decisions. Otherwise exits silently.

set -euo pipefail

# Ensure whatidid is available
if ! command -v whatidid &>/dev/null; then
    exit 0
fi

# Read the hook input from stdin
input="$(cat)"

# Extract the transcript path from the JSON
transcript_path="$(echo "$input" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    # The stop hook receives the transcript file path
    print(data.get('transcript_path', data.get('stopHookInput', {}).get('transcript_path', '')))
except:
    print('')
" 2>/dev/null)"

# If no transcript path or it doesn't exist, exit silently
if [ -z "$transcript_path" ] || [ ! -f "$transcript_path" ]; then
    exit 0
fi

# Analyze the transcript for decision indicators and recordings
analysis="$(python3 -c "
import json, sys, re

transcript_path = '$transcript_path'

decision_keywords = [
    'decided to', 'chose to', 'trade-off', 'tradeoff',
    'approach was', 'alternative was', 'opted for',
    'weighed the options', 'considered using',
    'went with', 'selected .* over', 'rejected',
    'instead of using', 'architectural decision',
    'design decision'
]

try:
    with open(transcript_path, 'r') as f:
        content = f.read()

    lines = content.strip().split('\n')

    decision_mentions = 0
    decision_records = 0

    for line in lines:
        try:
            entry = json.loads(line)
        except:
            continue

        role = entry.get('role', '')
        msg = ''
        if isinstance(entry.get('content'), str):
            msg = entry['content']
        elif isinstance(entry.get('content'), list):
            for block in entry['content']:
                if isinstance(block, dict) and block.get('type') == 'text':
                    msg += block.get('text', '')
                elif isinstance(block, dict) and block.get('type') == 'tool_use':
                    msg += json.dumps(block.get('input', {}))

        msg_lower = msg.lower()

        if role == 'assistant':
            for kw in decision_keywords:
                if re.search(kw, msg_lower):
                    decision_mentions += 1
                    break

        # Check for decision recording commands
        if 'whatidid page create' in msg and '--type decision' in msg:
            decision_records += 1

    # Only flag if we see decision language but no recordings
    # Be conservative: require at least 2 decision mentions
    if decision_mentions >= 2 and decision_records == 0:
        print('UNRECORDED')
    else:
        print('OK')
except Exception as e:
    print('OK')
" 2>/dev/null)"

if [ "$analysis" = "UNRECORDED" ]; then
    python3 -c "
import json
msg = '''It looks like decisions were discussed in this session but not recorded in whatidid.

Please record any non-trivial decisions before ending the session:

\`\`\`bash
whatidid page create --space <project-slug> --title \"Decision title\" --type decision --agent claude-code --labels \"topic1,topic2\" --body \"\$(cat <<'EOF'
## Context
Why this decision was needed.

## Options Considered
1. Option A
2. Option B

## Decision
What was chosen and why.

## Consequences
Trade-offs and implications.
EOF
)\"
\`\`\`

This ensures future sessions can learn from decisions made here.'''
print(json.dumps({'decision': 'block', 'reason': msg}))
"
else
    exit 0
fi
