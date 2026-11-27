#!/usr/bin/env bash
# ScoutChain — seed testnet with demo data.
# Run after initialize.sh to create test players, validators, and scouts.
#
# Minimum Funded Balance:
# Each seeded account requires a minimum balance of ~15 XLM to cover Stellar
# base reserves (~1-2 XLM), contract invocation gas fees (~0.01 XLM),
# subscription purchases (up to 7 XLM for Elite tier), and pay-to-contact
# fees (0.1 XLM per contact). Friendbot's standard testnet funding of 10,000 XLM
# per account is comfortably sufficient for the full demo flow.
#
# Idempotent: re-running this script is safe. Key generation, Friendbot
# funding, and contract registrations are all guarded against duplicate
# attempts. Existing registrations are detected and skipped gracefully.
#
# Exits non-zero immediately if any unrecoverable step fails (set -euo pipefail).
set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() { echo "ERROR: $*" >&2; exit 1; }

require_nonempty() {
  local value="$1"
  local label="$2"
  [[ -n "$value" ]] || die "$label is empty. Re-run deploy.sh and initialize.sh first."
}

# ensure_key <name>
#   Creates a named Stellar key if it does not already exist.
#   Uses --no-fund so the key is just stored locally; Friendbot funds it later.
ensure_key() {
  local name="$1"
  if ! stellar keys show "$name" &>/dev/null; then
    stellar keys generate --no-fund "$name"
  fi
}

# wait_for_account <address>
#   Polls the Soroban RPC until the account exists on the ledger, with up to
#   10 retries at 3-second intervals (30 seconds total). Friendbot funding is
#   asynchronous; invoking contracts before the account is active causes an
#   obscure "account not found" error. This guard eliminates that race.
wait_for_account() {
  local addr="$1"
  local max_attempts=10
  local wait_secs=3
  for i in $(seq 1 "$max_attempts"); do
    if stellar account show "$addr" --network "$NETWORK" &>/dev/null; then
      echo "    Account $addr is active on the ledger."
      return 0
    fi
    echo "    Waiting for $addr to appear on the ledger... ($i/$max_attempts)"
    sleep "$wait_secs"
  done
  die "Account $addr was not funded after $((max_attempts * wait_secs))s. Check Friendbot and network connectivity."
}

# invoke_idempotent <description> <error_fragment> <stellar_args...>
#   Invokes a contract function and treats a specific error as "already done"
#   (idempotent). Any other failure is fatal.
#
#   $1 — Human-readable description of the operation (for log messages)
#   $2 — Substring of the error output that signals "already registered / done"
#         (e.g. "AlreadyRegistered"). If the invocation fails with this string
#         in stderr, the error is swallowed and we continue; otherwise we die.
#   $3+ — Full stellar contract invoke command and arguments
invoke_idempotent() {
  local description="$1"
  local already_done_fragment="$2"
  shift 2

  local stderr_tmp
  stderr_tmp=$(mktemp)

  echo "==> $description..."
  if "$@" 2>"$stderr_tmp"; then
    rm -f "$stderr_tmp"
    echo "    Done."
    return 0
  fi

  local exit_code=$?
  local stderr_content
  stderr_content=$(cat "$stderr_tmp")
  rm -f "$stderr_tmp"

  if [[ -n "$already_done_fragment" ]] && echo "$stderr_content" | grep -q "$already_done_fragment"; then
    echo "    (already done — skipping)"
    return 0
  fi

  echo "ERROR: $description failed (exit $exit_code):" >&2
  echo "$stderr_content" >&2
  exit "$exit_code"
}

# ---------------------------------------------------------------------------
# Pre-flight: validate .env.contracts
# ---------------------------------------------------------------------------

[[ -f .env.contracts ]] || die ".env.contracts not found. Run ./scripts/deploy.sh and ./scripts/initialize.sh first."

# shellcheck source=/dev/null
source .env.contracts

require_nonempty "${REGISTRATION_CONTRACT_ID:-}"  "REGISTRATION_CONTRACT_ID"
require_nonempty "${VERIFICATION_CONTRACT_ID:-}"  "VERIFICATION_CONTRACT_ID"

NETWORK="testnet"
DEPLOYER="${DEPLOYER_SECRET:?Set DEPLOYER_SECRET in .env or environment}"
# ADMIN_ADDRESS is validated here even though not used directly in seed.sh —
# its presence confirms the .env is fully configured before continuing.
: "${ADMIN_ADDRESS:?Set ADMIN_ADDRESS in .env or environment}"

# ---------------------------------------------------------------------------
# Generate (or reuse) test keypairs
# ---------------------------------------------------------------------------

echo "==> Ensuring test keypairs exist..."
ensure_key player-test
ensure_key scout-test
ensure_key scout-test-2
ensure_key validator-test
ensure_key validator-test-2

PLAYER_ADDRESS=$(stellar keys address player-test)
SCOUT_ADDRESS=$(stellar keys address scout-test)
SCOUT_2_ADDRESS=$(stellar keys address scout-test-2)
VALIDATOR_ADDRESS=$(stellar keys address validator-test)
VALIDATOR_2_ADDRESS=$(stellar keys address validator-test-2)

echo "    Player:    $PLAYER_ADDRESS"
echo "    Scout:     $SCOUT_ADDRESS"
echo "    Scout 2:   $SCOUT_2_ADDRESS"
echo "    Validator: $VALIDATOR_ADDRESS"
echo "    Validator 2: $VALIDATOR_2_ADDRESS"

# ---------------------------------------------------------------------------
# Fund via Friendbot (safe to call multiple times — already-funded accounts
# receive an HTTP 400 which we ignore)
# ---------------------------------------------------------------------------

echo "==> Funding test accounts via Friendbot..."

fund_account() {
  local addr="$1"
  curl -sf "https://friendbot.stellar.org?addr=$addr" > /dev/null 2>&1 \
    || echo "    (account $addr may already be funded — continuing)"
}

fund_account "$PLAYER_ADDRESS"
fund_account "$SCOUT_ADDRESS"
fund_account "$SCOUT_2_ADDRESS"
fund_account "$VALIDATOR_ADDRESS"
fund_account "$VALIDATOR_2_ADDRESS"

# ---------------------------------------------------------------------------
# Wait for accounts to be active on the ledger before invoking contracts.
# Friendbot requests are asynchronous; without this guard the first contract
# invocation may fail with an obscure "account not found" error.
# ---------------------------------------------------------------------------

echo "==> Waiting for funded accounts to appear on the ledger..."
wait_for_account "$PLAYER_ADDRESS"
wait_for_account "$SCOUT_ADDRESS"
wait_for_account "$SCOUT_2_ADDRESS"
wait_for_account "$VALIDATOR_ADDRESS"
wait_for_account "$VALIDATOR_2_ADDRESS"

# ---------------------------------------------------------------------------
# Seed contract state (idempotent — each call is safe to re-run)
# ---------------------------------------------------------------------------

invoke_idempotent \
  "Registering validator" \
  "AlreadyRegistered" \
  stellar contract invoke \
    --id "$VERIFICATION_CONTRACT_ID" \
    --source "$DEPLOYER" \
    --network "$NETWORK" \
    -- register_validator \
    --wallet "$VALIDATOR_ADDRESS" \
    --credentials "UEFA B License — Test Validator" \
    --affiliation "Test Academy"

invoke_idempotent \
  "Registering second validator" \
  "AlreadyRegistered" \
  stellar contract invoke \
    --id "$VERIFICATION_CONTRACT_ID" \
    --source "$DEPLOYER" \
    --network "$NETWORK" \
    -- register_validator \
    --wallet "$VALIDATOR_2_ADDRESS" \
    --credentials "FIFA Talent ID — Test Validator 2" \
    --affiliation "Global Scouting Network"

invoke_idempotent \
  "Registering test player" \
  "AlreadyRegistered" \
  stellar contract invoke \
    --id "$REGISTRATION_CONTRACT_ID" \
    --source player-test \
    --network "$NETWORK" \
    -- register_player \
    --wallet "$PLAYER_ADDRESS" \
    --vitals '{"age":19,"position":"Forward","region":"West Africa","nationality":"Ghana"}' \
    --ipfs_hashes '["QmTestHighlight1","QmTestPhoto1"]'

invoke_idempotent \
  "Registering test scout" \
  "AlreadyRegistered" \
  stellar contract invoke \
    --id "$REGISTRATION_CONTRACT_ID" \
    --source scout-test \
    --network "$NETWORK" \
    -- register_scout \
    --wallet "$SCOUT_ADDRESS" \
    --region "Europe"

invoke_idempotent \
  "Registering second test scout" \
  "AlreadyRegistered" \
  stellar contract invoke \
    --id "$REGISTRATION_CONTRACT_ID" \
    --source scout-test-2 \
    --network "$NETWORK" \
    -- register_scout \
    --wallet "$SCOUT_2_ADDRESS" \
    --region "North America"

# ---------------------------------------------------------------------------
# Write .accounts file
# ---------------------------------------------------------------------------

ACCOUNTS_FILE="testnet/.accounts"
{
  echo "PLAYER_ADDRESS=$PLAYER_ADDRESS"
  echo "SCOUT_ADDRESS=$SCOUT_ADDRESS"
  echo "SCOUT_2_ADDRESS=$SCOUT_2_ADDRESS"
  echo "VALIDATOR_ADDRESS=$VALIDATOR_ADDRESS"
  echo "VALIDATOR_2_ADDRESS=$VALIDATOR_2_ADDRESS"
} > "$ACCOUNTS_FILE"

echo ""
echo "==> Seed complete."
echo "    Player address:    $PLAYER_ADDRESS"
echo "    Scout address:     $SCOUT_ADDRESS"
echo "    Scout 2 address:   $SCOUT_2_ADDRESS"
echo "    Validator address: $VALIDATOR_ADDRESS"
echo "    Validator 2 address: $VALIDATOR_2_ADDRESS"
echo "    Saved to $ACCOUNTS_FILE"
