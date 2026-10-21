#!/usr/bin/env bash
# ScoutChain — initialize all deployed contracts
# Run after deploy.sh. Requires .env.contracts to exist.
set -euo pipefail

source "$(dirname "$0")/network.sh" "${1:-}"
source .env.contracts

ADMIN="${ADMIN_ADDRESS:?Set ADMIN_ADDRESS}"
DEPLOYER="${DEPLOYER_SECRET:?Set DEPLOYER_SECRET}"
XLM_TOKEN="${XLM_TOKEN_ADDRESS:?Set XLM_TOKEN_ADDRESS}"

echo "==> Initializing registration contract..."
stellar contract invoke \
  --id "$REGISTRATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing verification contract..."
stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing progress contract..."
stellar contract invoke \
  --id "$PROGRESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing scout_access contract..."
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

echo "==> Wiring verification → progress cross-contract link..."
stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --progress_contract "$PROGRESS_CONTRACT_ID"

echo ""
echo "==> All contracts initialized and wired."
