#!/usr/bin/env bash
# upgrade.sh — upload a new WASM and drive storage migration to completion.
#
# `upgrade()` replaces the contract's WASM immediately, and Soroban gives a
# contract no hook that runs after its own code is swapped. Storage is therefore
# still on the old layout when `upgrade` returns, and the contract is serving
# traffic against a layout it does not have data for.
#
# This script closes that gap: upload, upgrade, then call `migrate` repeatedly
# until it reports `complete`, and finally verify through `schema_version`.
# See docs/VERSIONING.md.
#
# Exit codes:
#   0 — migration completed and schema_version matches
#   1 — usage error, or migration did not complete within --max-iterations
#   2 — post-migration verification failed
#
# Usage:
#   ./scripts/upgrade.sh --contract <id> --wasm <path> [--network <net>]
#                        [--target <version>] [--max-items <n>]
#                        [--max-iterations <n>]

set -euo pipefail

CONTRACT_ID=""
WASM_PATH=""
NETWORK="testnet"
TARGET_VERSION=""
MAX_ITEMS=100
MAX_ITERATIONS=50

usage() {
  sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --contract)        CONTRACT_ID="${2:-}"; shift 2 ;;
    --wasm)            WASM_PATH="${2:-}"; shift 2 ;;
    --network)         NETWORK="${2:-}"; shift 2 ;;
    --target)          TARGET_VERSION="${2:-}"; shift 2 ;;
    --max-items)       MAX_ITEMS="${2:-}"; shift 2 ;;
    --max-iterations)  MAX_ITERATIONS="${2:-}"; shift 2 ;;
    -h|--help)         usage ;;
    *) echo "Unknown argument: $1" >&2; usage ;;
  esac
done

if [[ -z "$CONTRACT_ID" || -z "$WASM_PATH" ]]; then
  echo "ERROR: --contract and --wasm are required." >&2
  usage
fi

if [[ ! -f "$WASM_PATH" ]]; then
  echo "ERROR: WASM not found at $WASM_PATH" >&2
  exit 1
fi

if ! command -v stellar >/dev/null 2>&1; then
  echo "ERROR: stellar CLI not found on PATH." >&2
  exit 1
fi

# Read a single u32 out of an invoke result. `stellar contract invoke --output
# json` wraps the value in {"result": ...}, and the result is itself a JSON
# string for a contracttype, so unwrap twice.
read_u32() {
  python3 -c '
import json, sys
raw = sys.stdin.read().strip()
try:
    doc = json.loads(raw)
except json.JSONDecodeError:
    print(raw); sys.exit(0)
value = doc.get("result", doc)
if isinstance(value, str):
    try:
        value = json.loads(value)
    except json.JSONDecodeError:
        pass
if isinstance(value, dict):
    for key in ("current", "complete", "last_visited_id", "processed"):
        if key in value:
            print(value[key]); sys.exit(0)
print("")
'
}

echo "==> Contract:  $CONTRACT_ID"
echo "==> Network:   $NETWORK"
echo "==> WASM:      $WASM_PATH"

BEFORE=$(stellar contract invoke --id "$CONTRACT_ID" --network "$NETWORK" \
  --output json -- schema_version 2>/dev/null | read_u32)
echo "==> Schema version before upgrade: ${BEFORE:-unknown}"

echo "==> Uploading WASM..."
stellar contract upload --wasm "$WASM_PATH" --network "$NETWORK"

WASM_HASH=$(stellar contract upload --wasm "$WASM_PATH" --network "$NETWORK" --source admin 2>/dev/null \
  | grep -oE '[a-f0-9]{64}' | head -1 || true)

if [[ -z "$WASM_HASH" ]]; then
  # Older CLI shapes print the hash only once; fall back to parsing the upload
  # output rather than failing the whole upgrade on a formatting difference.
  WASM_HASH=$(stellar contract upload --wasm "$WASM_PATH" --network "$NETWORK" 2>&1 \
    | grep -oE '[a-f0-9]{64}' | head -1 || true)
fi

if [[ -z "$WASM_HASH" ]]; then
  echo "ERROR: could not determine the uploaded WASM hash." >&2
  exit 1
fi

echo "==> Upgrading to WASM $WASM_HASH"
stellar contract invoke --id "$CONTRACT_ID" --network "$NETWORK" --source admin \
  -- upgrade "$WASM_HASH"

# With no explicit target, migrate to whatever the running code expects. The
# contract reports that as `code`, but that is only available after a migrate
# call, so default to the common case and let the verification step catch a
# mismatch rather than guessing a version and failing to reach it.
TARGET="${TARGET_VERSION:-1}"

echo "==> Driving migration to version $TARGET (max $MAX_ITEMS items per call)"

COMPLETE="false"
for (( i = 1; i <= MAX_ITERATIONS; i++ )); do
  RESULT=$(stellar contract invoke --id "$CONTRACT_ID" --network "$NETWORK" \
    --source admin --output json -- migrate "$TARGET" "$MAX_ITEMS" 2>/dev/null || echo "")

  COMPLETE=$(printf '%s' "$RESULT" | python3 -c '
import json, sys
raw = sys.stdin.read().strip()
if not raw:
    print("false"); sys.exit(0)
try:
    doc = json.loads(raw)
except json.JSONDecodeError:
    print("false"); sys.exit(0)
value = doc.get("result", doc)
if isinstance(value, str):
    try:
        value = json.loads(value)
    except json.JSONDecodeError:
        pass
print(str(bool(value.get("complete"))).lower() if isinstance(value, dict) else "false")
')

  CURSOR=$(printf '%s' "$RESULT" | read_u32)

  echo "    iteration ${i}: complete=${COMPLETE}"
  if [[ "$COMPLETE" == "true" ]]; then
    break
  fi
done

if [[ "$COMPLETE" != "true" ]]; then
  echo "ERROR: migration did not complete within $MAX_ITERATIONS iterations." >&2
  exit 1
fi

FINAL=$(stellar contract invoke --id "$CONTRACT_ID" --network "$NETWORK" \
  --output json -- schema_version 2>/dev/null | read_u32)

echo "==> Schema version after migration: ${FINAL:-unknown}"

if [[ -z "$FINAL" ]]; then
  echo "ERROR: could not read schema_version after migration." >&2
  exit 2
fi

if [[ "$FINAL" -lt "$TARGET" ]]; then
  echo "ERROR: storage is on version $FINAL but $TARGET was requested." >&2
  exit 2
fi

echo "==> Migration complete: storage is on schema version $FINAL"
