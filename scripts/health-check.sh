#!/usr/bin/env bash
# ScoutChain — post-deploy health check
# Calls health() on every deployed contract and asserts initialized: true, paused: false.
# Usage: ./scripts/health-check.sh [testnet|mainnet|local]
# Requires .env.contracts to exist (written by deploy.sh).
set -euo pipefail

NETWORK="${1:-testnet}"
# `stellar contract invoke` requires a --source-account even for read-only
# calls. Any funded account works for a simulation-only read; callers set
# STELLAR_SOURCE_ACCOUNT (an identity name or secret key), falling back to
# DEPLOYER_SECRET.
SOURCE_ACCOUNT="${STELLAR_SOURCE_ACCOUNT:-${DEPLOYER_SECRET:-}}"
if [[ -z "$SOURCE_ACCOUNT" ]]; then
  echo "ERROR: set STELLAR_SOURCE_ACCOUNT (or DEPLOYER_SECRET) to a Stellar identity or secret key." >&2
  exit 1
fi
# shellcheck source=/dev/null
source .env.contracts

CONTRACTS=(registration verification progress scout_access)

declare -A IDS=(
  [registration]="$REGISTRATION_CONTRACT_ID"
  [verification]="$VERIFICATION_CONTRACT_ID"
  [progress]="$PROGRESS_CONTRACT_ID"
  [scout_access]="$SCOUT_ACCESS_CONTRACT_ID"
)

FAILED=0

for name in "${CONTRACTS[@]}"; do
  id="${IDS[$name]}"
  echo "==> Checking health() on $name ($id)..."

  # stdout = the ContractHealth JSON; stellar-cli progress goes to stderr.
  response=$(stellar contract invoke \
    --id "$id" \
    --source "$SOURCE_ACCOUNT" \
    --network "$NETWORK" \
    -- health 2>/dev/null)

  echo "    Response: $response"

  # Parse the struct as JSON rather than grepping a whitespace-sensitive
  # substring — the CLI's formatting is not guaranteed to be compact.
  status=$(echo "$response" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
except Exception:
    print("UNPARSEABLE"); sys.exit()
if not d.get("initialized"):
    print("NOT_INITIALIZED")
elif d.get("paused"):
    print("PAUSED")
else:
    print("OK")
' 2>/dev/null || echo "UNPARSEABLE")

  case "$status" in
    OK)              echo "    OK: $name is healthy" ;;
    NOT_INITIALIZED) echo "    FAIL: $name returned initialized: false"; FAILED=1 ;;
    PAUSED)          echo "    FAIL: $name returned paused: true"; FAILED=1 ;;
    *)               echo "    FAIL: $name returned unexpected health response"; FAILED=1 ;;
  esac
done

if [[ "$FAILED" -ne 0 ]]; then
  echo ""
  echo "ERROR: One or more contracts failed the health check. See output above for details."
  exit 1
fi

echo ""
echo "==> All contracts are healthy."
