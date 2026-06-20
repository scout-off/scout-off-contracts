#!/usr/bin/env bash
# ScoutChain - initialize all deployed contracts.
# Run after deploy.sh. Requires .env.contracts to exist.
set -euo pipefail

NETWORK="${1:-testnet}"
source .env.contracts

ADMIN="${ADMIN_ADDRESS:?Set ADMIN_ADDRESS}"
DEPLOYER="${DEPLOYER_SECRET:?Set DEPLOYER_SECRET}"
XLM_TOKEN="${XLM_TOKEN_ADDRESS:?Set XLM_TOKEN_ADDRESS}"

invoke_or_skip_already_initialized() {
  local label="$1"
  shift
  local output

  set +e
  output="$("$@" 2>&1)"
  local status=$?
  set -e

  if [[ $status -eq 0 ]]; then
    printf '%s\n' "$output"
    return 0
  fi

  if grep -q "AlreadyInitialized" <<<"$output"; then
    echo "==> $label already initialized; continuing."
    return 0
  fi

  printf '%s\n' "$output" >&2
  return "$status"
}

wire_verification_progress() {
  local output

  set +e
  output="$(stellar contract invoke \
    --id "$VERIFICATION_CONTRACT_ID" \
    --source "$DEPLOYER" \
    --network "$NETWORK" \
    -- set_progress_contract \
    --progress_contract "$PROGRESS_CONTRACT_ID" 2>&1)"
  local status=$?
  set -e

  if [[ $status -eq 0 ]]; then
    printf '%s\n' "$output"
    return 0
  fi

  if grep -q "AlreadyConfigured" <<<"$output"; then
    echo "==> Verification progress link already configured; updating address instead..."
    stellar contract invoke \
      --id "$VERIFICATION_CONTRACT_ID" \
      --source "$DEPLOYER" \
      --network "$NETWORK" \
      -- update_progress_contract \
      --progress_contract "$PROGRESS_CONTRACT_ID"
    return 0
  fi

  printf '%s\n' "$output" >&2
  return "$status"
}

echo "==> Initializing registration contract..."
invoke_or_skip_already_initialized "registration contract" \
  stellar contract invoke \
  --id "$REGISTRATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing verification contract..."
invoke_or_skip_already_initialized "verification contract" \
  stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing progress contract..."
invoke_or_skip_already_initialized "progress contract" \
  stellar contract invoke \
  --id "$PROGRESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing scout_access contract..."
invoke_or_skip_already_initialized "scout_access contract" \
  stellar contract invoke \
  --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN" \
  --xlm_token "$XLM_TOKEN" \
  --fee_config '{
    "contact_fee_stroops": 1000000,
    "basic_sub_stroops": 10000000,
    "pro_sub_stroops": 30000000,
    "elite_sub_stroops": 70000000,
    "sub_duration_secs": 2592000
  }'

echo "==> Wiring verification -> progress cross-contract link..."
wire_verification_progress

echo ""
echo "==> All contracts initialized and wired."
