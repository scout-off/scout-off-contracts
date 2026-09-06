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

  response=$(stellar contract invoke \
    --id "$id" \
    --source "$SOURCE_ACCOUNT" \
    --network "$NETWORK" \
    -- health 2>&1)

  echo "    Response: $response"

  if echo "$response" | grep -q '"initialized":false'; then
    echo "    FAIL: $name returned initialized: false"
    FAILED=1
  elif echo "$response" | grep -q '"paused":true'; then
    echo "    FAIL: $name returned paused: true"
    FAILED=1
  elif echo "$response" | grep -q '"initialized":true'; then
    echo "    OK: $name is healthy"
  else
    echo "    FAIL: $name returned unexpected health response"
    FAILED=1
  fi
done

if [[ "$FAILED" -ne 0 ]]; then
  echo ""
  echo "ERROR: One or more contracts failed the health check. See output above for details."
  exit 1
fi

echo ""
echo "==> All contracts are healthy."
