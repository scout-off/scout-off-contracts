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

# Mainnet safety check: verify config file has no placeholders
if [[ "$NETWORK" == "mainnet" ]]; then
  if grep -q "FILL_IN_BEFORE_USE" config/mainnet.json; then
    echo "ERROR: config/mainnet.json contains placeholder values (FILL_IN_BEFORE_USE)"
    echo "Before deploying to mainnet, update config/mainnet.json with real values."
    exit 1
  fi
fi

# Keep WASM_DIR assigned exactly once; a duplicate legacy-target assignment can
# make deploys look in the wrong build directory.
WASM_DIR="target/wasm32v1-none/release"  # wasm32v1-none replaced the legacy wasm32-unknown-unknown target (soroban-sdk 25.x+)

# Save a pre-deploy snapshot so rollback.sh can restore the last known good state
if [[ -f ".env.contracts" ]]; then
  cp .env.contracts .env.contracts.snapshot
  echo "==> Pre-deploy snapshot saved to .env.contracts.snapshot"
fi

if command -v sha256sum >/dev/null 2>&1; then
  hash_wasm() { sha256sum "$1" | awk '{print $1}'; }
else
  hash_wasm() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

echo "==> Building contracts..."
cargo build --workspace --target wasm32v1-none --release

CONTRACTS=(registration verification progress scout_access)

declare -A CONTRACT_IDS
declare -A CONTRACT_WASM_HASHES

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
  CONTRACT_WASM_HASHES[$name]=$(hash_wasm "$optimized")
  echo "    $name => $id"
  echo "    $name wasm hash => ${CONTRACT_WASM_HASHES[$name]}"
done

# Write contract IDs and WASM hashes to .env.contracts
{
  echo "REGISTRATION_CONTRACT_ID=${CONTRACT_IDS[registration]}"
  echo "REGISTRATION_CONTRACT_WASM_HASH=${CONTRACT_WASM_HASHES[registration]}"
  echo "VERIFICATION_CONTRACT_ID=${CONTRACT_IDS[verification]}"
  echo "VERIFICATION_CONTRACT_WASM_HASH=${CONTRACT_WASM_HASHES[verification]}"
  echo "PROGRESS_CONTRACT_ID=${CONTRACT_IDS[progress]}"
  echo "PROGRESS_CONTRACT_WASM_HASH=${CONTRACT_WASM_HASHES[progress]}"
  echo "SCOUT_ACCESS_CONTRACT_ID=${CONTRACT_IDS[scout_access]}"
  echo "SCOUT_ACCESS_CONTRACT_WASM_HASH=${CONTRACT_WASM_HASHES[scout_access]}"
} > .env.contracts

# Write the same data as JSON for downstream tooling.
cat > .env.contracts.json <<EOF
{
  "network": "${NETWORK}",
  "contracts": {
    "registration": {
      "id": "${CONTRACT_IDS[registration]}",
      "wasm_hash": "${CONTRACT_WASM_HASHES[registration]}"
    },
    "verification": {
      "id": "${CONTRACT_IDS[verification]}",
      "wasm_hash": "${CONTRACT_WASM_HASHES[verification]}"
    },
    "progress": {
      "id": "${CONTRACT_IDS[progress]}",
      "wasm_hash": "${CONTRACT_WASM_HASHES[progress]}"
    },
    "scout_access": {
      "id": "${CONTRACT_IDS[scout_access]}",
      "wasm_hash": "${CONTRACT_WASM_HASHES[scout_access]}"
    }
  }
}
EOF

echo ""
echo "==> All contracts deployed. IDs saved to .env.contracts"
echo "    Pre-deploy snapshot is at .env.contracts.snapshot"
echo "    To roll back: ./scripts/rollback.sh $NETWORK"
