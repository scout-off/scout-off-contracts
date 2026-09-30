#!/usr/bin/env bash
# check-error-code-continuity.sh — CI lint step mechanically enforcing the
# append-only / no-renumbering error-code policy described in
# docs/CONTRIBUTING.md section "Error variant ordering".
#
# Usage:
#   bash scripts/check-error-code-continuity.sh [<base-ref>]
#
# <base-ref> defaults to "origin/main" (or "main" if origin/main is not
# reachable, e.g. in a local sandbox without a real remote).
#
# What it checks for every contract's errors.rs:
#   1. For each (code, variant) pair that exists in BASE, verify the SAME
#      code still maps to the SAME variant name in HEAD.
#      Fails if a numeric code was renamed.
#   2. For each code present in BASE but absent from HEAD, verify that the
#      HEAD source contains a "reserved" comment adjacent to that code number.
#      Fails if a code silently disappeared without a reservation annotation.
#   3. Recognises the existing ScoutAccessError code-13 reservation as a
#      valid exception: a gap whose source contains a comment with both the
#      word "reserved" and that code number is not a violation.
#
# Reuses the same error-code extraction regex as scripts/check-docs.sh.
#
# Exit codes:
#   0  no continuity violations found
#   1  one or more violations found (see output for details)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE_REF="${1:-origin/main}"

# Fall back to plain "main" when origin/main does not exist (detached-HEAD
# sandbox, first-run CI environment before the remote is fetched, etc.).
if ! git -C "$REPO_ROOT" rev-parse --verify "$BASE_REF" > /dev/null 2>&1; then
  BASE_REF="main"
fi
if ! git -C "$REPO_ROOT" rev-parse --verify "$BASE_REF" > /dev/null 2>&1; then
  echo "WARNING: base ref '$BASE_REF' not found; skipping continuity check."
  echo "         Pass a reachable ref as the first argument to enable this check."
  exit 0
fi

FAIL=0
WORK_DIR="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '$WORK_DIR'" EXIT

# ---------------------------------------------------------------------------
# Python comparator (written once, used by every contract below).
# Takes two arguments: path to base errors.rs and path to head errors.rs.
# Exits 0 (OK) or 1 (violation).
# ---------------------------------------------------------------------------
COMPARATOR="$WORK_DIR/compare_errors.py"
cat > "$COMPARATOR" <<'PYEOF'
import re
import sys

def extract_codes(path):
    """Return ({code_int: variant_name}, full_src) from a #[contracterror] enum."""
    src = open(path).read()
    m = re.search(r'#\[contracterror\].*?enum\s+\w+\s*\{([^}]+)\}', src, re.DOTALL)
    if not m:
        return {}, src
    body = m.group(1)
    result = {}
    for variant, code in re.findall(r'\b([A-Z][A-Za-z0-9]+)\s*=\s*(\d+)', body):
        result[int(code)] = variant
    return result, src

def has_reserved_comment(src, code):
    """Return True if src has a comment that mentions 'reserved' and the code number."""
    for line in src.splitlines():
        if re.search(r'(?i)reserved', line) and re.search(r'\b' + str(code) + r'\b', line):
            return True
    return False

base_map, _        = extract_codes(sys.argv[1])
head_map, head_src = extract_codes(sys.argv[2])

violations  = []
reserved_ok = []

for code, base_variant in sorted(base_map.items()):
    if code in head_map:
        head_variant = head_map[code]
        if base_variant != head_variant:
            violations.append(
                "Code {} renamed: was '{}', now '{}'"
                " -- renaming an existing error code is a breaking change"
                " (see docs/CONTRIBUTING.md, Error variant ordering)".format(
                    code, base_variant, head_variant)
            )
    else:
        # Code absent from HEAD: allowed only with a 'reserved' comment.
        if has_reserved_comment(head_src, code):
            reserved_ok.append(
                "  OK (reserved gap): code {} ('{}') absent with reservation"
                " comment -- treated as allowed exception".format(code, base_variant)
            )
        else:
            violations.append(
                "Code {} ('{}') removed without a 'reserved' comment"
                " -- either restore the variant or add a comment marking"
                " code {} as reserved"
                " (see docs/CONTRIBUTING.md, Error variant ordering)".format(
                    code, base_variant, code)
            )

for msg in reserved_ok:
    print(msg)

if violations:
    for v in violations:
        print("  VIOLATION: " + v)
    sys.exit(1)
else:
    print("  OK")
PYEOF

# ---------------------------------------------------------------------------
# check_one <label> <errors_rs_rel_path>
#   Fetches the BASE version into a temp file, then runs the comparator.
# ---------------------------------------------------------------------------
check_one() {
  local label="$1"
  local rel_path="$2"

  echo "Checking error-code continuity: $label"

  local base_tmp="$WORK_DIR/base_errors.rs"
  local head_path="$REPO_ROOT/$rel_path"

  if ! git -C "$REPO_ROOT" show "${BASE_REF}:${rel_path}" > "$base_tmp" 2>/dev/null; then
    echo "  SKIP: $rel_path not found in $BASE_REF (new contract?)"
    return 0
  fi

  if [[ ! -f "$head_path" ]]; then
    echo "  SKIP: $rel_path not present in working tree"
    return 0
  fi

  if ! python3 "$COMPARATOR" "$base_tmp" "$head_path"; then
    FAIL=1
  fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
echo "=== Error-code append-only / no-renumbering policy check ==="
echo "    Comparing working tree against: $BASE_REF"
echo ""

check_one "registration (ScoutChainError)" \
  "contracts/registration/src/errors.rs"

check_one "verification (VerificationError)" \
  "contracts/verification/src/errors.rs"

check_one "progress (ProgressError)" \
  "contracts/progress/src/errors.rs"

check_one "scout_access (ScoutAccessError)" \
  "contracts/scout_access/src/errors.rs"

echo ""
if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: Error-code continuity violation(s) found -- see above."
  echo "      Per docs/CONTRIBUTING.md: never renumber, never remove without"
  echo "      a 'reserved' comment. Append new variants at the end only."
  exit 1
else
  echo "PASS: All error codes are append-only and no codes were renamed."
fi
