#!/usr/bin/env bash
# ScoutChain — local sandbox setup (no testnet required).
# Starts (or reuses) a stellar/quickstart container, registers the `local`
# network, creates + funds an identity, then optionally deploys, initializes
# and seeds. Idempotent: re-running reuses the container and identity.
#
# Usage: ./scripts/setup-local.sh [--sandbox-only]
#   --sandbox-only  stop after the network/identity are ready (used by CI,
#                   which performs its own deploy steps).
set -euo pipefail

# Keep in sync with the image pinned in .github/workflows/contract-ci.yml.
QUICKSTART_IMAGE="${QUICKSTART_IMAGE:-stellar/quickstart:testing@sha256:66a19476f2ce2ed09c4dccf3f1af4d438209eee9ee05eb113a02dd926e2b03f5}"
CONTAINER="${LOCAL_CONTAINER:-stellar-local}"
IDENTITY="${STELLAR_SOURCE_ACCOUNT:-ci-deployer}"
RPC_URL="http://localhost:8000/soroban/rpc"
export NETWORK_PASSPHRASE="Standalone Network ; February 2017"
export STELLAR_SOURCE_ACCOUNT="$IDENTITY"

SANDBOX_ONLY=0
[[ "${1:-}" == "--sandbox-only" ]] && SANDBOX_ONLY=1

cd "$(dirname "$0")/.."

if docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER"; then
  if ! docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
    echo "Starting existing container $CONTAINER"
    docker start "$CONTAINER" >/dev/null
  else
    echo "Reusing running container $CONTAINER"
  fi
else
  docker run -d --name "$CONTAINER" -p 8000:8000 "$QUICKSTART_IMAGE" \
    --local --limits unlimited >/dev/null
fi

echo "Waiting for Soroban RPC to be ready..."
LEDGER=0
for i in $(seq 1 60); do
  LEDGER=$(curl -sf "$RPC_URL" -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' 2>/dev/null \
    | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('sequence',0))" \
      2>/dev/null || echo 0)
  if [ "${LEDGER:-0}" -gt 0 ] 2>/dev/null; then
    echo "RPC ready — ledger ${LEDGER} after ~$((i * 2))s"
    break
  fi
  sleep 2
done
if [ "${LEDGER:-0}" -eq 0 ] 2>/dev/null; then
  echo "ERROR: Soroban RPC never became ready" >&2
  docker logs "$CONTAINER" >&2 || true
  exit 1
fi

stellar network add local --rpc-url "$RPC_URL" \
  --network-passphrase "$NETWORK_PASSPHRASE"

if ! stellar keys address "$IDENTITY" >/dev/null 2>&1; then
  stellar keys generate "$IDENTITY" --network local
fi

# `keys generate --fund` swallows errors; fund separately and retry since
# friendbot can lag behind the RPC endpoint.
funded=0
for i in $(seq 1 30); do
  if stellar keys fund "$IDENTITY" --network local; then funded=1; break; fi
  echo "Friendbot not ready yet, retrying (${i}/30)..."
  sleep 2
done
[[ $funded -eq 1 ]] || { echo "ERROR: unable to fund $IDENTITY" >&2; exit 1; }

ADMIN_ADDRESS="$(stellar keys address "$IDENTITY")"
export ADMIN_ADDRESS
if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "ADMIN_ADDRESS=$ADMIN_ADDRESS" >> "$GITHUB_ENV"
fi

if [[ $SANDBOX_ONLY -eq 1 ]]; then
  echo "Sandbox ready (identity $IDENTITY)."
  exit 0
fi

if [[ -s .env.contracts ]] && grep -q 'CONTRACT_ID' .env.contracts 2>/dev/null; then
  echo "Existing deployment detected in .env.contracts; skipping deploy."
else
  cargo build --workspace --target wasm32v1-none --release --locked
  ./scripts/deploy.sh local
  ./scripts/initialize.sh local
fi
./testnet/seed.sh

echo "Local setup complete. Tear down with ./scripts/teardown-local.sh"
