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
  if [[ -z "$id" ]]; then
    echo "ERROR: Contract ID for $name is not set in .env.contracts." >&2
    exit 1
  fi

  echo "Checking health of $name ($id) on $NETWORK..."
  result=$(stellar contract invoke \
    --network "$NETWORK" \
    --source-account "$SOURCE_ACCOUNT" \
    --id "$id" \
    -- health)

  # Parse JSON result or check fields using python
  health_status=$(python3 -c '
import sys, json
try:
    raw = sys.stdin.read().strip()
    # Handle soroban json output formats
    if raw.startswith("{"):
        d = json.loads(raw)
    else:
        # Evaluate soroban scval or string representation if needed
        # Or parse standard json output from stellar-cli
        d = json.loads(raw)
    
    init = d.get("initialized", False)
    paused = d.get("paused", False)
    pay_paused = d.get("pay_to_contact_paused", False)
    mig_open = d.get("migration_window_open", False)
    
    if not init:
        print("NOT_INITIALIZED")
    elif paused:
        print("PAUSED")
    elif sys.argv[1] == "mainnet" and mig_open:
        print("MIGRATION_WINDOW_OPEN_MAINNET")
    else:
        print("OK")
except Exception as e:
    print(f"ERROR: {e}")
' "$NETWORK" <<< "$result")

  if [[ "$health_status" == "OK" ]]; then
    echo "    OK: $name is healthy."
  elif [[ "$health_status" == "NOT_INITIALIZED" ]]; then
    echo "    FAIL: $name is NOT initialized!" >&2
    FAILED=1
  elif [[ "$health_status" == "PAUSED" ]]; then
    echo "    FAIL: $name is PAUSED!" >&2
    FAILED=1
  elif [[ "$health_status" == "MIGRATION_WINDOW_OPEN_MAINNET" ]]; then
    echo "    FAIL: $name has migration window OPEN on mainnet!" >&2
    FAILED=1
  else
    echo "    FAIL: $name health check failed output: $result ($health_status)" >&2
    FAILED=1
  fi
done

if [[ $FAILED -ne 0 ]]; then
  echo "ERROR: One or more health checks failed." >&2
  exit 1
fi

echo "All health checks passed successfully."
