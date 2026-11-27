#!/usr/bin/env bash
# seed-integration-fixtures.sh — create the minimum fixtures the bindings
# example integration tests need (a player, a validator, a scout) on the local
# sandbox and write INTEGRATION_* variables to ${INTEGRATION_ENV_FILE:-integration.env}.
#
# Requires: .env.contracts (from deploy.sh), a running quickstart sandbox,
# and the ci-deployer identity (STELLAR_SOURCE_ACCOUNT, default ci-deployer).
set -euo pipefail

RPC_URL="${INTEGRATION_RPC_URL:-http://localhost:8000/soroban/rpc}"
FRIENDBOT_URL="${FRIENDBOT_URL:-http://localhost:8000/friendbot}"
DEPLOYER="${STELLAR_SOURCE_ACCOUNT:-ci-deployer}"
OUT="${INTEGRATION_ENV_FILE:-integration.env}"
NETWORK_ARGS=(--rpc-url "$RPC_URL" --network-passphrase "${NETWORK_PASSPHRASE:-Standalone Network ; February 2017}")

[[ -f .env.contracts ]] || { echo "ERROR: .env.contracts not found" >&2; exit 1; }
# shellcheck source=/dev/null
source .env.contracts

for name in it-player it-scout it-validator; do
  stellar keys show "$name" &>/dev/null || stellar keys generate --no-fund "$name"
  curl -sf "$FRIENDBOT_URL?addr=$(stellar keys address "$name")" >/dev/null || true
done
sleep 5

PLAYER_ADDRESS="$(stellar keys address it-player)"
SCOUT_ADDRESS="$(stellar keys address it-scout)"
VALIDATOR_ADDRESS="$(stellar keys address it-validator)"

invoke() { stellar contract invoke "${NETWORK_ARGS[@]}" "$@"; }

invoke --id "$VERIFICATION_CONTRACT_ID" --source "$DEPLOYER" -- register_validator \
  --wallet "$VALIDATOR_ADDRESS" --credentials "CI Validator" --affiliation "CI Academy" >&2

PLAYER_ID="$(invoke --id "$REGISTRATION_CONTRACT_ID" --source it-player -- register_player \
  --wallet "$PLAYER_ADDRESS" \
  --vitals '{"age":19,"position":"Forward","region":"West Africa","nationality":"Ghana"}' \
  --ipfs_hashes '["QmTestHighlight1","QmTestPhoto1"]' | tr -d '"[:space:]')"

invoke --id "$REGISTRATION_CONTRACT_ID" --source it-scout -- register_scout \
  --wallet "$SCOUT_ADDRESS" --region "Europe" >&2

{
  echo "INTEGRATION_RPC_URL=$RPC_URL"
  echo "INTEGRATION_NETWORK=local"
  echo "INTEGRATION_PLAYER_ID=$PLAYER_ID"
  echo "INTEGRATION_SECRET=$(stellar keys show it-player)"
  echo "INTEGRATION_SCOUT_SECRET=$(stellar keys show it-scout)"
  echo "INTEGRATION_VALIDATOR_SECRET=$(stellar keys show it-validator)"
  echo "INTEGRATION_TIER=Basic"
  echo "INTEGRATION_EVIDENCE_HASH=QmCiEvidenceHash"
} > "$OUT"
echo "==> Fixtures written to $OUT (player_id=$PLAYER_ID)"
