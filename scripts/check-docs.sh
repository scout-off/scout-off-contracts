#!/usr/bin/env bash
# check-docs.sh — CI lint step for CONTRACT_REFERENCE.md completeness.
#
# For every #[contractimpl] block in the four contracts this script extracts
# every `pub fn` name and verifies that a corresponding entry exists in
# docs/CONTRACT_REFERENCE.md.
#
# Exit codes:
#   0 — all public functions are documented
#   1 — one or more functions are missing from the reference

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCS_FILE="$REPO_ROOT/docs/CONTRACT_REFERENCE.md"
FAIL=0

# ---------------------------------------------------------------------------
# extract_pub_fns <file>
#   Prints each `pub fn` name found inside a #[contractimpl] block.
#   Uses a Python one-liner for portability (Python 3 is available on all
#   CI runners and macOS).
# ---------------------------------------------------------------------------
extract_pub_fns() {
  local file="$1"
  python3 - "$file" <<'PYEOF'
import re, sys

src = open(sys.argv[1]).read()

# Split around #[contractimpl] markers; we want the impl block that follows.
segments = re.split(r'#\[contractimpl\]', src)

for segment in segments[1:]:  # skip everything before the first marker
    depth = 0
    collecting = False
    i = 0
    block_chars = []

    # Skip whitespace/newlines then expect `impl ...`
    stripped = segment.lstrip()
    if not stripped.startswith('impl'):
        continue

    # Walk character-by-character to collect the impl block body
    for ch in segment:
        if ch == '{':
            depth += 1
            collecting = True
        elif ch == '}':
            depth -= 1
            if collecting and depth == 0:
                block_chars.append(ch)
                break
        if collecting:
            block_chars.append(ch)

    block = ''.join(block_chars)

    # Extract pub fn names (not private helpers — those lack `pub`)
    for m in re.finditer(r'\bpub fn ([a-z_][a-z0-9_]*)\b', block):
        print(m.group(1))
PYEOF
}

# ---------------------------------------------------------------------------
# check_contract <label> <src_file>
# ---------------------------------------------------------------------------
check_contract() {
  local label="$1"
  local src="$2"

  echo "Checking: $label"

  local missing=()
  while IFS= read -r fn_name; do
    # Accept either markdown heading style  #### `fn_name(`
    # or inline code span                   `fn_name(`
    if ! grep -qE "(####\s+\`${fn_name}\(|\`${fn_name}\()" "$DOCS_FILE"; then
      missing+=("$fn_name")
    fi
  done < <(extract_pub_fns "$src")

  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "  MISSING in CONTRACT_REFERENCE.md:"
    for fn in "${missing[@]}"; do
      echo "    - $fn"
    done
    FAIL=1
  else
    echo "  OK"
  fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
echo "=== CONTRACT_REFERENCE.md completeness check ==="
echo ""

check_contract "registration"  "$REPO_ROOT/contracts/registration/src/lib.rs"
check_contract "verification"  "$REPO_ROOT/contracts/verification/src/lib.rs"
check_contract "progress"      "$REPO_ROOT/contracts/progress/src/lib.rs"
check_contract "scout_access"  "$REPO_ROOT/contracts/scout_access/src/lib.rs"

echo ""
if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: One or more public functions are not documented in docs/CONTRACT_REFERENCE.md"
  echo "      Add an entry for each missing function and re-run this script."
  exit 1
else
  echo "PASS: All public contract functions are documented."
fi
