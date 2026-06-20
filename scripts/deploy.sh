#!/usr/bin/env bash
# ScoutChain — deploy all contracts to Stellar testnet or mainnet
# Usage: ./scripts/deploy.sh [testnet|mainnet]
set -euo pipefail

NETWORK="${1:-testnet}"
DEPLOYER="${DEPLOYER_SECRET:-}"

if [[ -z "$DEPLOYER" ]]; then
  echo "ERROR: Set DEPLOYER_SECRET env var to your Stellar secret key."
  exit 1
fi

WASM_DIR="target/wasm32v1-none/release"

echo "==> Building contracts..."
cargo build --workspace --target wasm32v1-none --release

CONTRACTS=(registration verification progress scout_access)

declare -A CONTRACT_IDS

for name in "${CONTRACTS[@]}"; do
  wasm_name="scoutchain_${name}.wasm"
  optimized="${WASM_DIR}/scoutchain_${name}.optimized.wasm"

  echo "==> Optimizing $name..."
  stellar contract optimize --wasm "${WASM_DIR}/${wasm_name}" --wasm-out "$optimized"

  echo "==> Deploying $name to $NETWORK..."
  id=$(stellar contract deploy \
    --wasm "$optimized" \
    --source "$DEPLOYER" \
    --network "$NETWORK")

  CONTRACT_IDS[$name]="$id"
  echo "    $name => $id"
done

# Write contract IDs to .env.contracts
{
  echo "REGISTRATION_CONTRACT_ID=${CONTRACT_IDS[registration]}"
  echo "VERIFICATION_CONTRACT_ID=${CONTRACT_IDS[verification]}"
  echo "PROGRESS_CONTRACT_ID=${CONTRACT_IDS[progress]}"
  echo "SCOUT_ACCESS_CONTRACT_ID=${CONTRACT_IDS[scout_access]}"
} > .env.contracts

echo ""
echo "==> All contracts deployed. IDs saved to .env.contracts"
