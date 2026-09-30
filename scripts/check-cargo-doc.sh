#!/usr/bin/env bash
# check-cargo-doc.sh — Run cargo doc across the workspace and report any
# missing-documentation warnings on public items.
#
# This is an opt-in local tool: it surfaces missing-doc warnings that would
# need individual triage.  Making #![deny(missing_docs)] a hard CI
# requirement repo-wide is a larger, separate undertaking.
#
# Usage:
#   bash scripts/check-cargo-doc.sh
#
# Exit codes:
#   0 — no missing-doc warnings found on public items
#   1 — one or more missing-doc warnings found (see output for details)
#   2 — cargo doc itself failed (compilation error, missing toolchain, etc.)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "=== cargo doc missing-documentation check ==="
echo "    Running: cargo doc --workspace --no-deps"
echo "    Lint flag: RUSTFLAGS=\"-W missing_docs\""
echo ""

# Run cargo doc with missing_docs warnings enabled at the rustc level.
# -W (warn) keeps the build going so all warnings surface, rather than
# stopping at the first missing doc like -D (deny) would.
STDERR_FILE="$(mktemp)"
# shellcheck disable=SC2064
trap "rm -f '$STDERR_FILE'" EXIT

set +e
(
  cd "$REPO_ROOT"
  RUSTFLAGS="-W missing_docs" \
    cargo doc --workspace --no-deps 2> "$STDERR_FILE"
)
CARGO_EXIT=$?
set -e

# If cargo doc itself failed, surface that separately from missing-doc warnings.
if [[ $CARGO_EXIT -ne 0 ]]; then
  echo "ERROR: cargo doc failed with exit code $CARGO_EXIT."
  echo "       Full stderr output follows:"
  echo "---"
  cat "$STDERR_FILE"
  echo "---"
  echo ""
  echo "Fix any compilation errors above before re-running this check."
  exit 2
fi

# Extract only the "missing documentation" warnings from stderr.
# Rust diagnostic lines start at column 0 with "warning:".
MISSING_DOCS=$(grep -E '^warning:.*missing documentation' "$STDERR_FILE" 2>/dev/null || true)

if [[ -z "$MISSING_DOCS" ]]; then
  echo "PASS: No missing-documentation warnings found on public items."
  exit 0
fi

# Count unique items missing docs.
COUNT=$(echo "$MISSING_DOCS" | wc -l | tr -d ' ')

echo "Found $COUNT missing-documentation warning(s) on public items:"
echo ""
echo "$MISSING_DOCS"
echo ""
echo "---"
echo "Full cargo doc stderr (may include other warnings):"
echo "---"
cat "$STDERR_FILE"

echo ""
echo "FAIL: $COUNT missing-documentation warning(s) found."
echo "      This is an opt-in local tool — fixing these warnings is not (yet)"
echo "      required for CI.  See docs/CONTRIBUTING.md for documentation"
echo "      conventions."
exit 1
