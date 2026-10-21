#!/usr/bin/env bash
# check-event-topic-consistency.sh — CI lint step verifying that every
# event-topic string emitted by each contract's events.rs has a matching
# row in that contract's Events table in docs/CONTRACT_REFERENCE.md.
#
# Strategy:
#   1. Extract every event-topic string from <contract>/src/events.rs by
#      scanning both forms used in the codebase:
#        a) String literals:  Symbol::new(env, "topic_name")
#        b) Named constants:  pub const FOO_EVENT_NAME: &str = "topic_name"
#            referenced via:  Symbol::new(env, CONST_NAME)
#      The Python extractor resolves constant references back to their
#      string values so the output is always the final topic string.
#   2. For each topic string, check that it appears — wrapped in backticks —
#      anywhere in the matching contract's Events section in
#      docs/CONTRACT_REFERENCE.md.  Diagnostic/deprecated events documented
#      under an explicit "Diagnostic Events" sub-heading also satisfy this
#      requirement.
#
# Running against the tree before the events-documentation-sync issue lands
# will immediately flag the four known-undocumented scout_access events
# (trial_offer_confirmed, trial_offer_expired, and any others) — which is
# the concrete validation that the script is working correctly.
#
# Exit codes:
#   0 — every emitted topic is documented
#   1 — one or more topics are missing from CONTRACT_REFERENCE.md

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCS_FILE="$REPO_ROOT/docs/CONTRACT_REFERENCE.md"
FAIL=0

# ---------------------------------------------------------------------------
# extract_event_topics <events_rs_file>
#   Prints each unique event-topic string found in the file.
#   Handles both:
#     - Symbol::new(env, "literal_name")
#     - pub const SOME_CONST: &str = "value"; used as Symbol::new(env, CONST)
# ---------------------------------------------------------------------------
extract_event_topics() {
  local file="$1"
  python3 - "$file" <<'PYEOF'
import re
import sys

src = open(sys.argv[1]).read()

# Step 1: collect all pub const <NAME>: &str = "<value>" declarations.
const_map = {}
for m in re.finditer(r'pub\s+const\s+([A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)"', src):
    const_map[m.group(1)] = m.group(2)

topics = set()

# Step 2: collect all Symbol::new(env, <arg>) usages.
for m in re.finditer(r'Symbol::new\s*\(\s*\w+\s*,\s*([^)]+?)\s*\)', src):
    arg = m.group(1).strip()
    # Case A: string literal  "topic_name"
    lit = re.match(r'^"([^"]+)"$', arg)
    if lit:
        topics.add(lit.group(1))
        continue
    # Case B: constant reference  SOME_CONST_NAME
    if arg in const_map:
        topics.add(const_map[arg])

for t in sorted(topics):
    print(t)
PYEOF
}

# ---------------------------------------------------------------------------
# extract_docs_section <contract_name>
#   Prints the block of CONTRACT_REFERENCE.md that belongs to <contract_name>.
#   We capture from the contract's "### <contract_name>" Events heading down
#   to the next "###" heading at the same depth (or end of file).
# ---------------------------------------------------------------------------
extract_docs_section() {
  local contract="$1"
  python3 - "$DOCS_FILE" "$contract" <<'PYEOF'
import re
import sys

content = open(sys.argv[1]).read()
contract = sys.argv[2]

# Match the per-contract Events sub-table.  The global ## Events section
# contains per-contract ### <name> sub-sections; we want the one matching
# our contract, all the way to the next ### heading.
pattern = r'###\s+' + re.escape(contract) + r'\s*\n(.*?)(?=\n###|\Z)'
m = re.search(pattern, content, re.DOTALL | re.IGNORECASE)
if m:
    print(m.group(1))
PYEOF
}

# ---------------------------------------------------------------------------
# check_contract_events <label> <contract_slug> <events_rs_path>
#   <label>        — human-readable name shown in output
#   <contract_slug> — name used in CONTRACT_REFERENCE.md Events heading
#   <events_rs_path> — path to the contract's events.rs file
# ---------------------------------------------------------------------------
check_contract_events() {
  local label="$1"
  local slug="$2"
  local events_rs="$3"

  echo "Checking events: $label"

  local section
  section=$(extract_docs_section "$slug")

  local missing=()
  while IFS= read -r topic; do
    [[ -z "$topic" ]] && continue
    # The topic must appear as `topic_name` (backtick-wrapped) anywhere in
    # the section (covers both the main table and any Diagnostic sub-table).
    if ! echo "$section" | grep -qF "\`${topic}\`"; then
      missing+=("$topic")
    fi
  done < <(extract_event_topics "$events_rs")

  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "  MISSING from CONTRACT_REFERENCE.md Events table:"
    for t in "${missing[@]}"; do
      echo "    - $t"
    done
    FAIL=1
  else
    echo "  OK"
  fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
echo "=== Event-topic / CONTRACT_REFERENCE.md consistency check ==="
echo ""

check_contract_events \
  "registration"  "registration" \
  "$REPO_ROOT/contracts/registration/src/events.rs"

check_contract_events \
  "verification"  "verification" \
  "$REPO_ROOT/contracts/verification/src/events.rs"

check_contract_events \
  "progress"  "progress" \
  "$REPO_ROOT/contracts/progress/src/events.rs"

check_contract_events \
  "scout_access"  "scout_access" \
  "$REPO_ROOT/contracts/scout_access/src/events.rs"

echo ""
if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: One or more event topics are missing from CONTRACT_REFERENCE.md."
  echo "      Add the missing rows to the matching Events table and re-run."
  exit 1
else
  echo "PASS: All event topics are documented in CONTRACT_REFERENCE.md."
fi
