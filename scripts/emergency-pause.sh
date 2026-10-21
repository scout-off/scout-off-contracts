#!/usr/bin/env bash
set -euo pipefail

# shellcheck disable=SC1091
source "$(dirname "$0")/../.env"
# shellcheck disable=SC1091
source "$(dirname "$0")/../.env.contracts"

NETWORK_ARGS=("--network" "$STELLAR_NETWORK" "--source" "$ADMIN_SECRET")

echo "==> Pausing registration contract..."
stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "==> Pausing verification contract..."
stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "==> Pausing progress contract..."
stellar contract invoke --id "$PROGRESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "==> Pausing scout_access contract..."
stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "All four contracts paused."
