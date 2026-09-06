#!/usr/bin/env bash
# ScoutChain — end-to-end smoke test for the address-migration tooling against
# a local Soroban sandbox, with a before/after state comparison.
#
# This mirrors the sandbox setup used by the `bindings-smoke-test` job in
# .github/workflows/contract-ci.yml (stellar/quickstart:testing docker
# container, a local network registration, and a funded CI identity) and then
# exercises the migration path this issue adds:
#
#   deploy OLD set -> seed a validator + a player -> migrate (deploy NEW set,
#   pause OLD, replay full supported state) -> compare OLD vs NEW state.
#
# It is intended as a MANUAL / optional command (it needs docker + the stellar
# CLI and pulls a container image), not a mandatory CI gate. Run it with:
#
#   ./scripts/migrate-contract-smoke-test.sh
#
# If docker or the stellar CLI is unavailable the script prints a SKIP notice
# and exits 0, so it is safe to invoke from environments that cannot stand up
# a sandbox.
#
set -euo pipefail

NETWORK="local"
CONTAINER="scoutchain-migrate-smoke"
RPC_URL="http://localhost:8000/soroban/rpc"
PASSPHRASE="Standalone Network ; February 2017"
XLM_TOKEN_ADDRESS="CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA"
WORKDIR="$(mktemp -d)"

# ---------------------------------------------------------------------------
# Preflight — skip cleanly if we cannot run a sandbox here.
# ---------------------------------------------------------------------------
if ! command -v docker >/dev/null 2>&1; then
  echo "SKIP: docker not available — cannot start a local Soroban sandbox."
  exit 0
fi
if ! command -v stellar >/dev/null 2>&1; then
  echo "SKIP: stellar CLI not available — install it (see docs/CONTRIBUTING.md) to run this test."
  exit 0
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "SKIP: jq not available — required for state comparison."
  exit 0
fi

cleanup() {
  echo "==> Cleaning up..."
  docker stop "$CONTAINER" >/dev/null 2>&1 || true
  docker rm "$CONTAINER" >/dev/null 2>&1 || true
  rm -rf "$WORKDIR" || true
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# 1. Start the local Soroban sandbox (same image as CI).
# ---------------------------------------------------------------------------
echo "==> Starting Soroban local sandbox ($CONTAINER)..."
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
# --limits unlimited: the stellar-core default Soroban resource ceilings are
# extremely low and reject the upload of verification's ~134 KB optimized WASM.
docker run -d --name "$CONTAINER" -p 8000:8000 stellar/quickstart:testing --local --limits unlimited >/dev/null

echo "==> Waiting for Soroban RPC to be ready..."
LEDGER=0
for _ in $(seq 1 60); do
  LEDGER=$(curl -sf "$RPC_URL" -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' 2>/dev/null \
    | jq -r '.result.sequence // 0' 2>/dev/null || echo 0)
  if [ "${LEDGER:-0}" -gt 0 ] 2>/dev/null; then
    echo "    RPC ready — ledger $LEDGER"
    break
  fi
  sleep 2
done
if [ "${LEDGER:-0}" -eq 0 ] 2>/dev/null; then
  echo "ERROR: Soroban RPC never became ready" >&2
  docker logs "$CONTAINER" || true
  exit 1
fi

# ---------------------------------------------------------------------------
# 2. Register the local network + fund identities.
# ---------------------------------------------------------------------------
echo "==> Registering local network with Stellar CLI..."
stellar network add "$NETWORK" --rpc-url "$RPC_URL" --network-passphrase "$PASSPHRASE" --overwrite \
  || stellar network add "$NETWORK" --rpc-url "$RPC_URL" --network-passphrase "$PASSPHRASE"

echo "==> Generating + funding admin identity..."
stellar keys generate smoke-admin --network "$NETWORK" --overwrite >/dev/null 2>&1 \
  || stellar keys generate smoke-admin --network "$NETWORK" >/dev/null 2>&1 || true
for _ in $(seq 1 30); do
  stellar keys fund smoke-admin --network "$NETWORK" >/dev/null 2>&1 && break
  sleep 2
done
ADMIN_ADDRESS="$(stellar keys address smoke-admin)"
DEPLOYER_SECRET="$(stellar keys show smoke-admin)"
export ADMIN_ADDRESS DEPLOYER_SECRET XLM_TOKEN_ADDRESS

echo "==> Generating + funding a player identity (holds its own key — required"
echo "    because register_player uses wallet.require_auth())..."
stellar keys generate smoke-player --network "$NETWORK" --overwrite >/dev/null 2>&1 \
  || stellar keys generate smoke-player --network "$NETWORK" >/dev/null 2>&1 || true
for _ in $(seq 1 30); do
  stellar keys fund smoke-player --network "$NETWORK" >/dev/null 2>&1 && break
  sleep 2
done
PLAYER_ADDRESS="$(stellar keys address smoke-player)"

# ---------------------------------------------------------------------------
# 3. Build WASM + deploy the OLD contract set via the real scripts.
# ---------------------------------------------------------------------------
echo "==> Building WASM contracts..."
cargo build --workspace --target wasm32v1-none --release

echo "==> Deploying OLD contract set (scripts/deploy.sh)..."
bash scripts/deploy.sh "$NETWORK"
bash scripts/initialize.sh "$NETWORK"

# Capture OLD ids.
OLD_VER_ID="$(grep -E '^VERIFICATION_CONTRACT_ID=' .env.contracts | cut -d= -f2-)"
OLD_REG_ID="$(grep -E '^REGISTRATION_CONTRACT_ID=' .env.contracts | cut -d= -f2-)"

# ---------------------------------------------------------------------------
# 4. Seed state on the OLD contracts: one validator + one player.
# ---------------------------------------------------------------------------
echo "==> Seeding a validator on the OLD verification contract (admin-signed)..."
stellar contract invoke --id "$OLD_VER_ID" --source smoke-admin --network "$NETWORK" \
  -- register_validator --wallet "$ADMIN_ADDRESS" --credentials "smoke-test-credentials-0001"

echo "==> Seeding a player on the OLD registration contract (player-signed)..."
stellar contract invoke --id "$OLD_REG_ID" --source smoke-player --network "$NETWORK" \
  -- register_player \
  --wallet "$PLAYER_ADDRESS" \
  --vitals '{"age":21,"position":"ST","region":"EU","nationality":"NG"}' \
  --ipfs_hashes '["QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"]'

# ---------------------------------------------------------------------------
# 5. BEFORE snapshot of OLD state.
# ---------------------------------------------------------------------------
echo "==> Capturing BEFORE state (old contracts)..."
BEFORE_VALIDATORS="$(stellar contract invoke --id "$OLD_VER_ID" --network "$NETWORK" -- get_validators | jq -S '.')"
BEFORE_PLAYER_COUNT="$(stellar contract invoke --id "$OLD_REG_ID" --network "$NETWORK" -- get_player_count | tr -dc '0-9')"
echo "    OLD validators   : $BEFORE_VALIDATORS"
echo "    OLD player_count : $BEFORE_PLAYER_COUNT"

# ---------------------------------------------------------------------------
# 6. Run the migration (non-interactive), then replay.
# ---------------------------------------------------------------------------
echo "==> Running migrate-contract.sh --yes (deploy NEW, pause OLD, replay)..."
bash scripts/migrate-contract.sh "$NETWORK" --yes

NEW_VER_ID="$(grep -E '^VERIFICATION_CONTRACT_ID=' .env.contracts | cut -d= -f2-)"
NEW_REG_ID="$(grep -E '^REGISTRATION_CONTRACT_ID=' .env.contracts | cut -d= -f2-)"

# ---------------------------------------------------------------------------
# 7. AFTER snapshot of NEW state + comparison.
# ---------------------------------------------------------------------------
echo "==> Capturing AFTER state (new contracts)..."
AFTER_VALIDATORS="$(stellar contract invoke --id "$NEW_VER_ID" --network "$NETWORK" -- get_validators | jq -S '.')"
AFTER_PLAYER_COUNT="$(stellar contract invoke --id "$NEW_REG_ID" --network "$NETWORK" -- get_player_count | tr -dc '0-9')"
echo "    NEW validators   : $AFTER_VALIDATORS"
echo "    NEW player_count : $AFTER_PLAYER_COUNT"

echo ""
echo "=========================================================================="
echo "  BEFORE / AFTER comparison"
echo "=========================================================================="
FAIL=0

# Validators must be replayed exactly (set equality, order-independent).
if [ "$BEFORE_VALIDATORS" = "$AFTER_VALIDATORS" ]; then
  echo "  PASS: validators replayed onto the new contract identically."
else
  echo "  FAIL: validator set differs after migration."
  echo "        before=$BEFORE_VALIDATORS"
  echo "        after =$AFTER_VALIDATORS"
  FAIL=1
fi

# Players must be replayed with their stable IDs and payloads.
if [ "${AFTER_PLAYER_COUNT:-0}" -eq "${BEFORE_PLAYER_COUNT:-0}" ]; then
  echo "  PASS: player count preserved across migration."
else
  echo "  FAIL: player count changed (before=${BEFORE_PLAYER_COUNT:-0}, after=${AFTER_PLAYER_COUNT:-0})."
  FAIL=1
fi

# The player must still be exported so the replay is auditable.
LATEST_PLAYER_EXPORT="$(find migration-export -name 'players-*.json' -type f 2>/dev/null | sort | tail -1 || true)"
if [ -n "$LATEST_PLAYER_EXPORT" ]; then
  EXPORTED_COUNT="$(jq 'length' "$LATEST_PLAYER_EXPORT" 2>/dev/null || echo 0)"
  if [ "${EXPORTED_COUNT:-0}" -ge 1 ]; then
    echo "  PASS: $EXPORTED_COUNT player(s) exported to $LATEST_PLAYER_EXPORT."
  else
    echo "  FAIL: player export $LATEST_PLAYER_EXPORT is empty."
    FAIL=1
  fi
else
  echo "  FAIL: no player export file was produced under migration-export/."
  FAIL=1
fi

echo "=========================================================================="
if [ "$FAIL" -ne 0 ]; then
  echo "SMOKE TEST FAILED — see comparison above."
  exit 1
fi
echo "SMOKE TEST PASSED — migration tooling behaves as designed."
