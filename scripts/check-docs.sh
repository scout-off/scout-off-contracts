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
echo "=== CONTRACT_REFERENCE.md error-code check ==="
python3 - "$REPO_ROOT" "$DOCS_FILE" <<'PYEOF' || FAIL=1
import re
import sys
from pathlib import Path

repo = Path(sys.argv[1])
docs_file = Path(sys.argv[2])

contracts = {
    "ScoutChainError": repo / "contracts/registration/src/errors.rs",
    "VerificationError": repo / "contracts/verification/src/errors.rs",
    "ProgressError": repo / "contracts/progress/src/errors.rs",
    "ScoutAccessError": repo / "contracts/scout_access/src/errors.rs",
}


def parse_enum(path: Path, enum_name: str) -> dict[int, str]:
    src = path.read_text()
    match = re.search(rf"pub enum {enum_name}\s*\{{(?P<body>.*?)\n\}}", src, re.S)
    if not match:
        raise SystemExit(f"missing enum {enum_name} in {path}")

    values: dict[int, str] = {}
    for variant, code in re.findall(r"\b([A-Za-z][A-Za-z0-9_]*)\s*=\s*(\d+)\s*,", match.group("body")):
        values[int(code)] = variant
    return values


def parse_doc_table(doc: str, enum_name: str) -> dict[int, str]:
    match = re.search(
        rf"### `{enum_name}`.*?\n(?P<section>.*?)(?:\n### |\n---|\Z)",
        doc,
        re.S,
    )
    if not match:
        raise SystemExit(f"missing docs section for {enum_name}")

    values: dict[int, str] = {}
    for code, variant in re.findall(r"^\|\s*(\d+)\s*\|\s*`([^`]+)`\s*\|", match.group("section"), re.M):
        values[int(code)] = variant
    return values


doc = docs_file.read_text()
failed = False
for enum_name, path in contracts.items():
    expected = parse_enum(path, enum_name)
    actual = parse_doc_table(doc, enum_name)
    if actual != expected:
        failed = True
        print(f"  MISMATCH: {enum_name}")
        missing = expected.keys() - actual.keys()
        extra = actual.keys() - expected.keys()
        changed = {code for code in expected.keys() & actual.keys() if expected[code] != actual[code]}
        for code in sorted(missing):
            print(f"    missing {code} = {expected[code]}")
        for code in sorted(extra):
            print(f"    extra {code} = {actual[code]}")
        for code in sorted(changed):
            print(f"    {code}: docs={actual[code]} source={expected[code]}")
    else:
        print(f"  OK: {enum_name}")

if failed:
    raise SystemExit(1)
PYEOF

echo ""
if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: One or more public functions are not documented in docs/CONTRACT_REFERENCE.md"
  echo "      Add an entry for each missing function and re-run this script."
  exit 1
else
  echo "PASS: All public contract functions are documented."
fi
