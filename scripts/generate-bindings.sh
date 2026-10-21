#!/usr/bin/env bash
# ScoutChain — generate TypeScript bindings for all contracts
# Usage: ./scripts/generate-bindings.sh [testnet|mainnet|local]
# Requires .env.contracts to exist (written by deploy.sh)
set -euo pipefail

source "$(dirname "$0")/network.sh" "${1:-}"
source .env.contracts

CONTRACTS=(registration verification progress scout_access)

declare -A IDS=(
  [registration]="$REGISTRATION_CONTRACT_ID"
  [verification]="$VERIFICATION_CONTRACT_ID"
  [progress]="$PROGRESS_CONTRACT_ID"
  [scout_access]="$SCOUT_ACCESS_CONTRACT_ID"
)

for name in "${CONTRACTS[@]}"; do
  id="${IDS[$name]}"
  out="bindings/${name}"

  echo "==> Generating TypeScript bindings for $name ($id)..."
  stellar contract bindings typescript \
    --contract-id "$id" \
    --network "$NETWORK" \
    --output-dir "$out" \
    --overwrite

  echo "    Written to $out/"
done

echo ""
echo "==> All bindings generated. Publish or link them into backend/frontend."
