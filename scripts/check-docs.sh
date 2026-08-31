#!/usr/bin/env bash
# check-docs.sh — CI lint step for CONTRACT_REFERENCE.md completeness.
#
# 1. For every #[contractimpl] block in the four contracts this script extracts
#    every `pub fn` name and verifies that a corresponding entry exists in
#    docs/CONTRACT_REFERENCE.md.
#
# 2. For every #[contracterror] enum in each errors.rs this script extracts
#    all `VariantName = N` discriminants and verifies that every (code, variant)
#    pair appears verbatim in the matching per-contract error table in
#    docs/CONTRACT_REFERENCE.md.  This catches numeric-code drift without
#    requiring a compiled WASM binary.
#
# Exit codes:
#   0 — all public functions and all error codes are documented correctly
#   1 — one or more entries are missing or have the wrong numeric code

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
# extract_error_codes <errors_rs_file>
#   Prints "CODE VARIANT" pairs from a #[contracterror] enum.
# ---------------------------------------------------------------------------
extract_error_codes() {
  local file="$1"
  python3 - "$file" <<'PYEOF'
import re, sys

src = open(sys.argv[1]).read()

# Locate the contracterror enum body
m = re.search(r'#\[contracterror\].*?enum\s+\w+\s*\{([^}]+)\}', src, re.DOTALL)
if not m:
    sys.exit(0)

body = m.group(1)
# Match lines like:  VariantName = 12,  (with optional doc comments before)
for match in re.finditer(r'\b([A-Z][A-Za-z0-9]+)\s*=\s*(\d+)', body):
    print(match.group(2), match.group(1))
PYEOF
}

# ---------------------------------------------------------------------------
# check_error_codes <label> <errors_rs_file> <section_header_pattern>
#   Verifies every (code, variant) pair from the Rust source is present
#   in CONTRACT_REFERENCE.md under the matching section heading.
# ---------------------------------------------------------------------------
check_error_codes() {
  local label="$1"
  local errors_rs="$2"
  local section_pattern="$3"

  echo "Checking error codes: $label"

  # Extract the relevant section from CONTRACT_REFERENCE.md
  local section
  section=$(python3 - "$DOCS_FILE" "$section_pattern" <<'PYEOF'
import re, sys

content = open(sys.argv[1]).read()
pattern = sys.argv[2]

# Find the section that matches the pattern, then grab text until the next ###
m = re.search(pattern + r'.*?\n(.*?)(?=\n###|\Z)', content, re.DOTALL | re.IGNORECASE)
if m:
    print(m.group(1))
PYEOF
)

  local missing=()
  local wrong_code=()

  while IFS=' ' read -r code variant; do
    [[ -z "$code" || -z "$variant" ]] && continue
    # Each row must contain the numeric code and the backtick-quoted variant name
    if ! echo "$section" | grep -qE "^\|\s*${code}\s*\|.*\`${variant}\`"; then
      # Distinguish: variant present but with wrong code vs entirely absent
      if echo "$section" | grep -qE "\`${variant}\`"; then
        wrong_code+=("${variant} (expected code ${code})")
      else
        missing+=("${code} = ${variant}")
      fi
    fi
  done < <(extract_error_codes "$errors_rs")

  if [[ ${#missing[@]} -eq 0 && ${#wrong_code[@]} -eq 0 ]]; then
    echo "  OK"
    return
  fi

  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "  MISSING from CONTRACT_REFERENCE.md:"
    for e in "${missing[@]}"; do echo "    - $e"; done
  fi
  if [[ ${#wrong_code[@]} -gt 0 ]]; then
    echo "  WRONG CODE in CONTRACT_REFERENCE.md:"
    for e in "${wrong_code[@]}"; do echo "    - $e"; done
  fi
  FAIL=1
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
echo "=== Error code drift check ==="
echo ""

check_error_codes "registration (ScoutChainError)" \
  "$REPO_ROOT/contracts/registration/src/errors.rs" \
  "### \`ScoutChainError\`"

check_error_codes "verification (VerificationError)" \
  "$REPO_ROOT/contracts/verification/src/errors.rs" \
  "### \`VerificationError\`"

check_error_codes "progress (ProgressError)" \
  "$REPO_ROOT/contracts/progress/src/errors.rs" \
  "### \`ProgressError\`"

check_error_codes "scout_access (ScoutAccessError)" \
  "$REPO_ROOT/contracts/scout_access/src/errors.rs" \
  "### \`ScoutAccessError\`"

# ---------------------------------------------------------------------------
# Cross-contract call drift check
#
# For each pub fn that makes a cross-contract call (detected via
# `_contract::Client::new` in the Rust source), the corresponding entry in
# CONTRACT_REFERENCE.md must contain a structured annotation line:
#
#   | **Cross-contract calls:** | <target>.<method> |
#
# If a function makes a cross-contract call but the doc entry has no such
# annotation, the check fails — catching the class of drift where a
# function's documented behaviour diverges from what the code actually does.
# ---------------------------------------------------------------------------

# extract_cross_contract_calls <lib_rs_file>
#   Prints "function_name target.method" pairs for every pub fn whose body
#   contains a `_contract::Client::new` call.
extract_cross_contract_calls() {
  local file="$1"
  python3 - "$file" <<'PYEOF'
import re, sys

src = open(sys.argv[1]).read()

# Remove block comments
src = re.sub(r'/\*.*?\*/', '', src, flags=re.DOTALL)
# Remove line comments
src = re.sub(r'//[^\n]*', '', src)

# Find #[contractimpl] block
segments = re.split(r'#\[contractimpl\]', src)

for segment in segments[1:]:
    stripped = segment.lstrip()
    if not stripped.startswith('impl'):
        continue

    # Walk the impl block body
    depth = 0
    collecting = False
    block_chars = []
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

    # Split into individual pub fn bodies
    fn_pattern = re.compile(r'\bpub fn ([a-z_][a-z0-9_]*)\b')
    matches = list(fn_pattern.finditer(block))

    for i, m in enumerate(matches):
        fn_name = m.group(1)
        fn_start = m.start()
        fn_end = matches[i + 1].start() if i + 1 < len(matches) else len(block)
        fn_body = block[fn_start:fn_end]

        # Detect cross-contract Client::new calls:
        # Pattern: something_contract::Client::new ... .method( or .try_method(
        client_pattern = re.compile(
            r'(\w+)_contract::Client::new[^;]*?'
            r'(?:\.(\w+)\s*\(|\.try_(\w+)\s*\()',
            re.DOTALL
        )
        calls_found = set()
        for cm in client_pattern.finditer(fn_body):
            target = cm.group(1)
            method = cm.group(2) or cm.group(3)
            if method and target:
                calls_found.add(f"{target}.{method}")

        for call in sorted(calls_found):
            print(f"{fn_name} {call}")
PYEOF
}

# check_cross_contract_calls <label> <lib_rs_file>
check_cross_contract_calls() {
  local label="$1"
  local src="$2"

  echo "Checking cross-contract calls: $label"

  local mismatches=()

  while IFS=' ' read -r fn_name call; do
    [[ -z "$fn_name" || -z "$call" ]] && continue

    # Find the function's section in CONTRACT_REFERENCE.md
    local fn_doc_section
    fn_doc_section=$(python3 - "$DOCS_FILE" "$fn_name" <<'PYEOF'
import re, sys

content = open(sys.argv[1]).read()
fn_name = sys.argv[2]

# Find the heading for this function
pattern = rf'####\s+`{re.escape(fn_name)}\('
m = re.search(pattern, content)
if not m:
    pattern = rf'####\s+{re.escape(fn_name)}\('
    m = re.search(pattern, content)
if not m:
    print("")
    sys.exit(0)

section_start = m.start()
next_heading = re.search(r'\n####', content[m.end():])
if next_heading:
    section_end = m.end() + next_heading.start()
else:
    section_end = len(content)

print(content[section_start:section_end])
PYEOF
)

    if [[ -z "$fn_doc_section" ]]; then
      # Function not in docs — covered by the function-existence check above
      continue
    fi

    # Check that the doc section has a Cross-contract calls annotation for
    # this specific call target.method
    local target method
    target=$(echo "$call" | cut -d. -f1)
    method=$(echo "$call" | cut -d. -f2)

    if ! echo "$fn_doc_section" | grep -qiE "\*\*Cross-contract calls:\*\*.*${target}\.${method}"; then
      mismatches+=("${fn_name}: code calls ${call} but doc has no '**Cross-contract calls:** ${call}' annotation")
    fi
  done < <(extract_cross_contract_calls "$src")

  if [[ ${#mismatches[@]} -gt 0 ]]; then
    echo "  CROSS-CONTRACT CALL DRIFT:"
    for m_item in "${mismatches[@]}"; do
      echo "    - $m_item"
    done
    FAIL=1
  else
    echo "  OK"
  fi
}

echo ""
echo "=== Cross-contract call drift check ==="
echo ""

check_cross_contract_calls "verification"  "$REPO_ROOT/contracts/verification/src/lib.rs"
check_cross_contract_calls "progress"      "$REPO_ROOT/contracts/progress/src/lib.rs"
check_cross_contract_calls "scout_access"  "$REPO_ROOT/contracts/scout_access/src/lib.rs"
check_cross_contract_calls "registration"  "$REPO_ROOT/contracts/registration/src/lib.rs"

echo ""
# --- Sanity check: ensure CONTRACT_REFERENCE.md is not truncated ---
DOC_LINES=$(wc -l < "$DOCS_FILE")
DOC_LAST_LINE=$(tail -1 "$DOCS_FILE")
if [[ "$DOC_LAST_LINE" != *"# END OF DOCS CONTRACT_REFERENCE.md"* ]]; then
  echo "  WARNING: DOCS FILE LAST LINE UNEXPECTED:"
  echo "    Last line: $DOC_LAST_LINE"
  echo "    (Expected marker: '# END OF DOCS CONTRACT_REFERENCE.md')"
  echo "    Doc file may be truncated — verify CONTRACT_REFERENCE.md completeness."
  FAIL=1
fi
if [[ $DOC_LINES -lt 100 ]]; then
  echo "  WARNING: DOCS FILE UNEXPECTEDLY SHORT ($DOC_LINES lines):"
  echo "    CONTRACT_REFERENCE.md may be truncated."
  FAIL=1
fi
echo ""

if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: One or more issues found — see above."
  echo "      Update docs/CONTRACT_REFERENCE.md to match the Rust source and re-run."
  exit 1
else
  echo "PASS: All public functions, error codes, and cross-contract calls are correctly documented."
fi
