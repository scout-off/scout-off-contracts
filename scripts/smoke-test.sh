#!/usr/bin/env bash
# ScoutChain — cross-contract smoke test for approve_milestone → advance_level
#
# Deploys verification and progress contracts to testnet, wires them, and
# exercises the real cross-contract call: approve_milestone on verification
# must atomically advance the player's level in the progress contract.
#
# Usage: ./scripts/smoke-test.sh
# Requires: DEPLOYER_SECRET, ADMIN_ADDRESS env vars; stellar CLI installed.
set -euo pipefail

NETWORK="${1:-testnet}"
DEPLOYER="${DEPLOYER_SECRET:?Set DEPLOYER_SECRET}"
ADMIN="${ADMIN_ADDRESS:?Set ADMIN_ADDRESS}"

WASM_DIR="target/wasm32v1-none/release"
VER_WASM="${WASM_DIR}/scoutchain_verification.optimized.wasm"
PROG_WASM="${WASM_DIR}/scoutchain_progress.optimized.wasm"

echo "============================================"
echo "  ScoutChain Cross-Contract Smoke Test"
echo "============================================"

# 1. Build contracts
echo ""
echo "==> Building contracts..."
cargo build --workspace --target wasm32v1-none --release

echo "==> Optimizing wasm..."
stellar contract optimize --wasm "${WASM_DIR}/scoutchain_verification.wasm" \
  --wasm-out "$VER_WASM"
stellar contract optimize --wasm "${WASM_DIR}/scoutchain_progress.wasm" \
  --wasm-out "$PROG_WASM"

# 2. Deploy contracts
echo ""
echo "==> Deploying verification contract..."
VER_ID=$(stellar contract deploy --wasm "$VER_WASM" --source "$DEPLOYER" --network "$NETWORK")
echo "    verification => $VER_ID"

echo "==> Deploying progress contract..."
PROG_ID=$(stellar contract deploy --wasm "$PROG_WASM" --source "$DEPLOYER" --network "$NETWORK")
echo "    progress => $PROG_ID"

# 3. Initialize contracts
echo ""
echo "==> Initializing verification contract..."
stellar contract invoke \
  --id "$VER_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- initialize --admin "$ADMIN"

echo "==> Initializing progress contract..."
stellar contract invoke \
  --id "$PROG_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- initialize --admin "$ADMIN"

# 4. Wire verification → progress
echo ""
echo "==> Wiring verification -> progress..."
stellar contract invoke \
  --id "$VER_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- set_progress_contract \
  --progress_contract "$PROG_ID"

# 5. Wire progress → verification (so advance_level validates the caller)
echo "==> Wiring progress -> verification..."
stellar contract invoke \
  --id "$PROG_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- set_verification_contract \
  --addr "$VER_ID"

# 6. Register a validator
echo ""
echo "==> Registering validator..."
# Use ADMIN as the validator wallet for simplicity
stellar contract invoke \
  --id "$VER_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- register_validator \
  --wallet "$ADMIN" \
  --credentials "Smoke Test Coach" \
  --affiliation "Smoke Test Academy" \
  --specializations '[]'

# 7. Approve a milestone (this triggers the cross-contract call)
echo ""
echo "==> Approving milestone (cross-contract call)..."
stellar contract invoke \
  --id "$VER_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- approve_milestone \
  --validator_wallet "$ADMIN" \
  --player_id 1 \
  --description "Smoke test milestone" \
  --evidence_hash "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"

# 8. Verify that the player's level advanced in the progress contract
echo ""
echo "==> Verifying cross-contract call succeeded..."
LEVEL=$(stellar contract invoke \
  --id "$PROG_ID" --source "$DEPLOYER" --network "$NETWORK" \
  -- get_level \
  --player_id 1)

echo "    Player 1 level => $LEVEL"

if echo "$LEVEL" | grep -qi "VerifiedIdentity"; then
  echo ""
  echo "============================================"
  echo "  SMOKE TEST PASSED"
  echo "  approve_milestone → advance_level wiring OK"
  echo "============================================"
else
  echo ""
  echo "============================================"
  echo "  SMOKE TEST FAILED"
  echo "  Expected VerifiedIdentity, got: $LEVEL"
  echo "============================================"
  exit 1
fi
