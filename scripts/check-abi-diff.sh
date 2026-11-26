#!/usr/bin/env bash
# ScoutChain — ABI-diff CI gate
#
# Compares the PR branch's abi/*-abi.json against main's and classifies
# each contract's change severity per docs/VERSIONING.md. Fails if a
# MAJOR or MINOR change lacks a matching CHANGELOG.md entry.
#
# Usage (called from CI):
#   ./scripts/check-abi-diff.sh [base_ref]
#
# Arguments:
#   base_ref   Git ref to diff against (default: origin/main)
#
# Environment:
#   GITHUB_BASE_REF   Set automatically by GitHub Actions for pull_request events.
#   GITHUB_HEAD_REF   Set automatically by GitHub Actions for pull_request events.

set -euo pipefail

BASE_REF="${1:-${GITHUB_BASE_REF:-origin/main}}"
HEAD_REF="${GITHUB_HEAD_REF:-HEAD}"
CONTRACTS=(registration verification progress scout_access)
ABI_DIR="abi"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
  echo "FAIL: $*" >&2
  exit 1
}

# Extract function names from an ABI JSON file.
# Returns a sorted newline-separated list of "contract_name::function_name(params)".
extract_functions() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo ""
    return
  fi
  python3 - <<'PYEOF' "$file"
import json, sys
with open(sys.argv[1]) as f:
    abi = json.load(f)
for fn in abi.get("functions", []):
    name = fn.get("name", "")
    params = fn.get("params", [])
    param_types = ", ".join(p.get("type", "") for p in params)
    print(f"{name}({param_types})")
PYEOF
}

# Classify the diff between two sorted function lists.
# Outputs: MAJOR, MINOR, PATCH, or NO_CHANGE
classify_diff() {
  local base_file="$1"
  local head_file="$2"

  if [[ ! -f "$base_file" ]]; then
    echo "MINOR"
    return
  fi

  local base_fns head_fns
  base_fns=$(extract_functions "$base_file" | sort)
  head_fns=$(extract_functions "$head_file" | sort)

  local base_count head_count
  base_count=$(echo "$base_fns" | grep -c . || true)
  head_count=$(echo "$head_fns" | grep -c . || true)

  if [[ "$base_fns" == "$head_fns" ]]; then
    echo "NO_CHANGE"
    return
  fi

  # Check for removed functions (MAJOR)
  local removed
  removed=$(comm -23 <(echo "$base_fns") <(echo "$head_fns") || true)
  if [[ -n "$removed" ]]; then
    echo "MAJOR"
    return
  fi

  # Check for added functions (MINOR)
  local added
  added=$(comm -13 <(echo "$base_fns") <(echo "$head_fns") || true)
  if [[ -n "$added" ]]; then
    echo "MINOR"
    return
  fi

  # Parameter changes are MAJOR
  echo "MAJOR"
}

# Check CHANGELOG.md for a matching entry.
# Arguments: contract_name classification
check_changelog() {
  local contract="$1"
  local classification="$2"
  local changelog="CHANGELOG.md"

  if [[ ! -f "$changelog" ]]; then
    echo "    WARNING: CHANGELOG.md not found"
    return 0
  fi

  # Look for Unreleased section entries mentioning the contract and classification
  local found
  found=$(python3 - <<'PYEOF' "$contract" "$classification" "$changelog"
import json, sys, re
contract = sys.argv[1]
classification = sys.argv[2].upper()
with open(sys.argv[3]) as f:
    content = f.read()

# Extract Unreleased section
m = re.search(r'## Unreleased\b(.*?)(?=\n## |\Z)', content, re.DOTALL)
if not m:
    print("false")
    sys.exit(0)

unreleased = m.group(1)
# Check if the contract is mentioned in any entry
contract_mentioned = contract.lower() in unreleased.lower()
# Check if the classification is mentioned
class_mentioned = classification.lower() in unreleased.lower()
print("true" if (contract_mentioned and class_mentioned) else "false")
PYEOF
)

  if [[ "$found" == "true" ]]; then
    echo "    CHANGELOG: OK (entry found for $contract / $classification)"
  else
    echo "    CHANGELOG: MISSING — no Unreleased entry found for $contract ($classification)"
    return 1
  fi
}

# Check docs/VERSIONING.md for a matching Version History row added in the diff.
# Requires a row in the Version History table for MAJOR/MINOR ABI changes.
check_version_history() {
  local base_ref="$1"
  local head_ref="$2"
  local versioning="docs/VERSIONING.md"

  if [[ ! -f "$versioning" ]]; then
    echo "    VERSIONING: MISSING — docs/VERSIONING.md not found"
    return 1
  fi

  local added_rows
  added_rows=$(git diff --no-ext-diff --unified=0 "$base_ref" "$head_ref" -- "$versioning" 2>/dev/null || true)

  if echo "$added_rows" | grep -Eq '^\+\|[[:space:]]*v[0-9]+\.[0-9]+\.[0-9]+([[:space:]]*\([^)]+\))?[[:space:]]*\|'; then
    echo "    VERSIONING: OK (Version History row added)"
    return 0
  fi

  echo "    VERSIONING: MISSING — no Version History row added for this MAJOR/MINOR change"
  return 1
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

echo "==> ABI diff gate: comparing HEAD ($HEAD_REF) against $BASE_REF"
echo ""

# Build the current branch's ABI if not already present
if [[ ! -d "$ABI_DIR" ]] || [[ -z "$(ls -A "$ABI_DIR" 2>/dev/null)" ]]; then
  echo "==> Building WASM and exporting ABIs..."
  mkdir -p "$ABI_DIR"
  WASM=target/wasm32v1-none/release
  for contract in "${CONTRACTS[@]}"; do
    wasm="${WASM}/scoutchain_${contract}.wasm"
    if [[ ! -f "$wasm" ]]; then
      echo "    Building $contract..."
      cargo build -p scoutchain-${contract} --target wasm32v1-none --release
    fi
    echo "    Exporting $contract ABI..."
    stellar contract info interface --wasm "$wasm" --output json-formatted > "$ABI_DIR/${contract}-abi.json"
  done
fi

# Fetch the base ref's ABI files into a temp directory
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "==> Fetching base ABI from $BASE_REF..."
git show "$BASE_REF:abi/${CONTRACTS[0]}-abi.json" > "$TMPDIR/${CONTRACTS[0]}-abi.json" 2>/dev/null || true
git show "$BASE_REF:abi/${CONTRACTS[1]}-abi.json" > "$TMPDIR/${CONTRACTS[1]}-abi.json" 2>/dev/null || true
git show "$BASE_REF:abi/${CONTRACTS[2]}-abi.json" > "$TMPDIR/${CONTRACTS[2]}-abi.json" 2>/dev/null || true
git show "$BASE_REF:abi/${CONTRACTS[3]}-abi.json" > "$TMPDIR/${CONTRACTS[3]}-abi.json" 2>/dev/null || true

echo ""
echo "==> Diff results:"
echo ""

overall_fail=0

for contract in "${CONTRACTS[@]}"; do
  base_file="$TMPDIR/${contract}-abi.json"
  head_file="$ABI_DIR/${contract}-abi.json"

  if [[ ! -f "$head_file" ]]; then
    echo "  $contract: SKIP (no head ABI found)"
    continue
  fi

  if [[ ! -f "$base_file" ]]; then
    echo "  $contract: MINOR (new contract — base ABI not found)"
    if ! check_changelog "$contract" "MINOR"; then
      overall_fail=1
    fi
    if ! check_version_history "$BASE_REF" "$HEAD_REF"; then
      overall_fail=1
    fi
    continue
  fi

  classification=$(classify_diff "$base_file" "$head_file")
  echo "  $contract: $classification"

  if [[ "$classification" == "MAJOR" ]] || [[ "$classification" == "MINOR" ]]; then
    if ! check_changelog "$contract" "$classification"; then
      overall_fail=1
    fi
    if ! check_version_history "$BASE_REF" "$HEAD_REF"; then
      overall_fail=1
    fi
  fi
done

echo ""
if [[ "$overall_fail" -ne 0 ]]; then
  die "ABI-diff gate failed: see CHANGELOG.md warnings above."
fi

echo "==> ABI-diff gate passed."
