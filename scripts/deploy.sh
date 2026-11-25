#!/usr/bin/env bash
# ScoutChain — deploy all contracts to Stellar testnet or mainnet
# Usage: ./scripts/deploy.sh [testnet|mainnet|local] [--allow-cli-mismatch]
set -euo pipefail

NETWORK_ARG=""
ALLOW_CLI_MISMATCH=0
for arg in "$@"; do
  case "$arg" in
    --allow-cli-mismatch) ALLOW_CLI_MISMATCH=1 ;;
    *) NETWORK_ARG="$arg" ;;
  esac
done

# shellcheck disable=SC1091
source "$(dirname "$0")/network.sh" "$NETWORK_ARG"

# shellcheck disable=SC1091
source "$(dirname "${BASH_SOURCE[0]}")/lib/check-stellar-cli.sh"
check_stellar_cli_version "$ALLOW_CLI_MISMATCH"
DEPLOYER="${DEPLOYER_SECRET:-}"

if [[ -z "$DEPLOYER" ]]; then
  echo "ERROR: Set DEPLOYER_SECRET env var to your Stellar secret key."
  exit 1
fi

# Contracts initialize in their constructors, atomically with deployment
# (#1369), so the admin and scout_access's token/fee config are needed here.
ADMIN="${ADMIN_ADDRESS:?Set ADMIN_ADDRESS (the admin passed to every constructor)}"
CONFIG_FILE="config/${NETWORK}.json"
if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "ERROR: $CONFIG_FILE not found."
  echo "Create it (see docs/CONFIG_REFERENCE.md) or check the network name."
  exit 1
fi

# The local network has no fixed native-asset contract address: derive it from
# the running quickstart unless the operator supplied XLM_TOKEN_ADDRESS.
if [[ "$NETWORK" == "local" && -z "${XLM_TOKEN_ADDRESS:-}" ]]; then
  echo "==> Deriving local native asset contract address..."
  if ! XLM_TOKEN_ADDRESS=$(stellar contract id asset --asset native --network local 2>/dev/null) \
     || [[ -z "$XLM_TOKEN_ADDRESS" ]]; then
    echo "ERROR: Could not derive the local native asset contract address."
    echo "Make sure a local Stellar network is running, or set XLM_TOKEN_ADDRESS explicitly."
    exit 1
  fi
  echo "    native asset => $XLM_TOKEN_ADDRESS"
fi

CONFIG_OUT=$(python3 "$(dirname "$0")/lib/read_network_config.py" "$CONFIG_FILE" "${XLM_TOKEN_ADDRESS:-}") || exit 1
read -r XLM_TOKEN _REG_COOLDOWN_SECS FEE_CONFIG_JSON <<<"$CONFIG_OUT"
=======

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

# Save a pre-deploy snapshot so rollback.sh can restore the last known good
# state. Snapshots are timestamped and never overwritten, so consecutive
# deploys keep a full history; .env.contracts.snapshot mirrors the latest one
# for backward compatibility.
SNAPSHOT_DIR=".env.contracts.snapshots"
if [[ -f ".env.contracts" ]]; then
  mkdir -p "$SNAPSHOT_DIR"
  SNAPSHOT_TS=$(date -u +%Y%m%dT%H%M%SZ)
  SNAPSHOT_BASE="${SNAPSHOT_DIR}/${SNAPSHOT_TS}-${NETWORK}"
  cp .env.contracts "${SNAPSHOT_BASE}.env"
  if [[ -f ".env.contracts.json" ]]; then
    cp .env.contracts.json "${SNAPSHOT_BASE}.json"
  fi
  cp .env.contracts .env.contracts.snapshot
  echo "==> Pre-deploy snapshot saved to ${SNAPSHOT_BASE}.env"
  echo "    Latest snapshot mirrored to .env.contracts.snapshot"
fi
=======

if command -v sha256sum >/dev/null 2>&1; then
  hash_wasm() { sha256sum "$1" | awk '{print $1}'; }
else
  hash_wasm() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

# Optional: deploy prebuilt release WASMs (e.g. downloaded from a GitHub
# Release, see docs/DEPLOYMENT.md) instead of building locally. The directory
# must hold scoutchain_<contract>.optimized.wasm for every contract.
PREBUILT_WASM_DIR="${PREBUILT_WASM_DIR:-}"
if [[ -n "$PREBUILT_WASM_DIR" ]]; then
  WASM_DIR="$PREBUILT_WASM_DIR"
  echo "==> Using prebuilt WASMs from $WASM_DIR (skipping build and optimize)"
else
  echo "==> Building contracts..."
  cargo build --workspace --target wasm32v1-none --release --locked
fi

CONTRACTS=(registration verification progress scout_access)

declare -A CONTRACT_IDS
declare -A CONTRACT_WASM_HASHES

for name in "${CONTRACTS[@]}"; do
  wasm_name="scoutchain_${name}.wasm"
  optimized="${WASM_DIR}/scoutchain_${name}.optimized.wasm"

  if [[ -z "$PREBUILT_WASM_DIR" ]]; then
    echo "==> Optimizing $name..."
    stellar contract optimize --wasm "${WASM_DIR}/${wasm_name}" --wasm-out "$optimized"
  elif [[ ! -f "$optimized" ]]; then
    echo "ERROR: $optimized not found in PREBUILT_WASM_DIR"; exit 1
  fi

  if [[ "$name" == "scout_access" ]]; then
    ctor_args=(--admin "$ADMIN" --xlm_token "$XLM_TOKEN" --fee_config "$FEE_CONFIG_JSON")
  else
    ctor_args=(--admin "$ADMIN")
  fi

  echo "==> Deploying $name to $NETWORK..."
  id=$(stellar contract deploy \
    --wasm "$optimized" \
    --source "$DEPLOYER" \
    --network "$NETWORK" \
    -- "${ctor_args[@]}")

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

# Gather deployment metadata for .env.contracts.json. Never write secrets:
# deployer_public_key is derived from the identity, not the secret key.
GIT_COMMIT=$(git rev-parse HEAD 2>/dev/null || echo "unknown")
if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
  GIT_DIRTY="true"
else
  GIT_DIRTY="false"
fi
STELLAR_CLI_VERSION=$(stellar --version 2>/dev/null | head -n1 || echo "unknown")
DEPLOYER_PUBLIC_KEY=$(stellar keys address "$DEPLOYER" 2>/dev/null || echo "")
DEPLOYED_AT=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-}"

# Best-effort latest ledger sequence from RPC; empty if unavailable.
LATEST_LEDGER=""
if [[ -n "${SOROBAN_RPC_URL:-}" ]]; then
  LATEST_LEDGER=$(curl -s -X POST "$SOROBAN_RPC_URL" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' 2>/dev/null \
    | python3 -c 'import json,sys
try:
    print(json.load(sys.stdin)["result"]["sequence"])
except Exception:
    pass' 2>/dev/null || true)
fi

# Write the same data as JSON for downstream tooling, including a meta object.
NETWORK="$NETWORK" \
GIT_COMMIT="$GIT_COMMIT" \
GIT_DIRTY="$GIT_DIRTY" \
STELLAR_CLI_VERSION="$STELLAR_CLI_VERSION" \
DEPLOYER_PUBLIC_KEY="$DEPLOYER_PUBLIC_KEY" \
NETWORK_PASSPHRASE="$NETWORK_PASSPHRASE" \
DEPLOYED_AT="$DEPLOYED_AT" \
LATEST_LEDGER="$LATEST_LEDGER" \
PREBUILT_WASM_DIR="$PREBUILT_WASM_DIR" \
REGISTRATION_ID="${CONTRACT_IDS[registration]}" \
REGISTRATION_HASH="${CONTRACT_WASM_HASHES[registration]}" \
VERIFICATION_ID="${CONTRACT_IDS[verification]}" \
VERIFICATION_HASH="${CONTRACT_WASM_HASHES[verification]}" \
PROGRESS_ID="${CONTRACT_IDS[progress]}" \
PROGRESS_HASH="${CONTRACT_WASM_HASHES[progress]}" \
SCOUT_ACCESS_ID="${CONTRACT_IDS[scout_access]}" \
SCOUT_ACCESS_HASH="${CONTRACT_WASM_HASHES[scout_access]}" \
python3 - > .env.contracts.json <<'PYEOF'
import json
import os

env = os.environ

meta = {
    "git_commit": env["GIT_COMMIT"],
    "git_dirty": env["GIT_DIRTY"] == "true",
    "stellar_cli_version": env["STELLAR_CLI_VERSION"],
    "deployer_public_key": env["DEPLOYER_PUBLIC_KEY"],
    "network_passphrase": env["NETWORK_PASSPHRASE"],
    "deployed_at": env["DEPLOYED_AT"],
}
if env.get("LATEST_LEDGER"):
    try:
        meta["latest_ledger"] = int(env["LATEST_LEDGER"])
    except ValueError:
        pass
if env.get("PREBUILT_WASM_DIR"):
    meta["prebuilt_wasm_dir"] = env["PREBUILT_WASM_DIR"]

data = {
    "network": env["NETWORK"],
    "meta": meta,
    "contracts": {
        "registration": {
            "id": env["REGISTRATION_ID"],
            "wasm_hash": env["REGISTRATION_HASH"],
        },
        "verification": {
            "id": env["VERIFICATION_ID"],
            "wasm_hash": env["VERIFICATION_HASH"],
        },
        "progress": {
            "id": env["PROGRESS_ID"],
            "wasm_hash": env["PROGRESS_HASH"],
        },
        "scout_access": {
            "id": env["SCOUT_ACCESS_ID"],
            "wasm_hash": env["SCOUT_ACCESS_HASH"],
        },
    },
}

print(json.dumps(data, indent=2))
PYEOF

echo ""
echo "==> All contracts deployed. IDs saved to .env.contracts"
echo "    Snapshots are kept in $SNAPSHOT_DIR (latest mirrored to .env.contracts.snapshot)"
echo "    To list snapshots: ./scripts/rollback.sh --list"
echo "    To roll back: ./scripts/rollback.sh $NETWORK"
=======
