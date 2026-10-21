#!/usr/bin/env bash
# check-bindings.sh — CI lint step for bindings package.json templates.
#
# Verifies that every bindings/{contract}/package.json:
#   - Is valid JSON
#   - Has a non-empty "name" field
#   - Has a "scripts.build" entry
#   - Declares "@stellar/stellar-sdk" as a dependency
#
# Exit codes:
#   0 — all templates are valid
#   1 — one or more templates are invalid
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FAIL=0

check_package_json() {
  local pkg="$1"
  local file="$REPO_ROOT/bindings/$pkg/package.json"

  echo "Checking bindings/$pkg/package.json..."

  if [[ ! -f "$file" ]]; then
    echo "  MISSING: package.json not found"
    FAIL=1
    return
  fi

  python3 - "$file" <<'PYEOF'
import json, sys

path = sys.argv[1]
try:
    p = json.load(open(path))
except json.JSONDecodeError as e:
    print(f"  INVALID JSON: {e}")
    sys.exit(1)

errors = []

if not p.get("name"):
    errors.append("missing 'name' field")

scripts = p.get("scripts", {})
if not scripts.get("build"):
    errors.append("missing 'scripts.build' field")

deps = {**p.get("dependencies", {}), **p.get("peerDependencies", {})}
if "@stellar/stellar-sdk" not in deps:
    errors.append("missing '@stellar/stellar-sdk' in dependencies or peerDependencies")

if errors:
    for e in errors:
        print(f"  ERROR: {e}")
    sys.exit(1)

print(f"  OK: {p['name']}")
PYEOF
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    FAIL=1
  fi
}

echo "=== Bindings package.json validation ==="
echo ""

for pkg in registration verification progress scout_access; do
  check_package_json "$pkg"
done

echo ""
if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: One or more bindings/package.json templates are invalid."
  exit 1
else
  echo "PASS: All bindings package.json templates are valid."
fi
