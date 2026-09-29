#!/usr/bin/env bash
# ScoutChain — upgrade a deployed contract WASM in-place.
#
# Usage:
#   ./scripts/upgrade.sh <network> <contract_name> <wasm_path>
#
# Arguments:
#   network        testnet | mainnet
#   contract_name  registration | verification | progress | scout_access
#   wasm_path      Path to the compiled .wasm file
#                  (e.g. target/wasm32v1-none/release/scoutchain_registration.wasm)
#
# Prerequisites:
#   • .env.contracts must exist (written by deploy.sh)
#   • ADMIN_ADDRESS and DEPLOYER_SECRET must be set in .env or environment
#   • The DEPLOYER_SECRET must be the keypair for ADMIN_ADDRESS (same check
#     as initialize.sh)
#
# What this script does:
#   1. Verifies the signing keypair matches ADMIN_ADDRESS (fail-fast guard).
#   2. Snapshots on-chain instance state that needs to be re-applied after
#      a WASM swap (fee config, contract links, version).
#   3. Installs the new WASM via `stellar contract install` and captures the
#      new wasm hash.
#   4. Calls `upgrade(new_wasm_hash)` on the target contract via the admin key.
#   5. Calls `health()` to confirm the contract is still responsive.
#   6. Prints a post-upgrade checklist reminding the operator to re-apply any
#      instance-storage values that may need restoring.
#
# Upgrading does NOT change the contract ID — all existing clients, bindings,
# and off-chain indexes continue to reference the same address.  The persistent
# storage (player profiles, subscriptions, validator registry …) is untouched.
# Only the executable WASM is replaced.
#
# After upgrading, regenerate TypeScript bindings if the ABI changed:
#   ./scripts/generate-bindings.sh <network>
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Argument validation
# ---------------------------------------------------------------------------
if [[ $# -lt 3 ]]; then
  echo "Usage: $0 <network> <contract_name> <wasm_path>" >&2
  echo "  network        testnet | mainnet" >&2
  echo "  contract_name  registration | verification | progress | scout_access" >&2
  echo "  wasm_path      path to .wasm file" >&2
  exit 1
fi

NETWORK="$1"
CONTRACT_NAME="$2"
WASM_PATH="$3"

case "$NETWORK" in
  testnet|mainnet) ;;
  *)
    echo "ERROR: unknown network '$NETWORK'. Use 'testnet' or 'mainnet'." >&2
    exit 1
    ;;
esac

VALID_CONTRACTS="registration verification progress scout_access"
if ! echo "$VALID_CONTRACTS" | grep -qw "$CONTRACT_NAME"; then
  echo "ERROR: unknown contract '$CONTRACT_NAME'." >&2
  echo "       Valid names: $VALID_CONTRACTS" >&2
  exit 1
fi

if [[ ! -f "$WASM_PATH" ]]; then
  echo "ERROR: WASM file not found: $WASM_PATH" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Load environment
# ---------------------------------------------------------------------------
# shellcheck source=/dev/null
[[ -f .env ]] && source .env
# shellcheck source=/dev/null
[[ -f .env.contracts ]] && source .env.contracts

ADMIN="${ADMIN_ADDRESS:?ADMIN_ADDRESS is not set}"
DEPLOYER="${DEPLOYER_SECRET:?DEPLOYER_SECRET is not set}"

# Map contract name → contract ID variable
case "$CONTRACT_NAME" in
  registration) CONTRACT_ID="${REGISTRATION_CONTRACT_ID:?REGISTRATION_CONTRACT_ID not set}" ;;
  verification) CONTRACT_ID="${VERIFICATION_CONTRACT_ID:?VERIFICATION_CONTRACT_ID not set}" ;;
  progress)     CONTRACT_ID="${PROGRESS_CONTRACT_ID:?PROGRESS_CONTRACT_ID not set}" ;;
  scout_access) CONTRACT_ID="${SCOUT_ACCESS_CONTRACT_ID:?SCOUT_ACCESS_CONTRACT_ID not set}" ;;
esac

# ---------------------------------------------------------------------------
# Guard: signing keypair must match ADMIN_ADDRESS
# (same guard as initialize.sh — prevents locking the contract to a phantom admin)
# ---------------------------------------------------------------------------
echo "==> Verifying admin keypair..."
DERIVED_ADMIN=$(stellar keys address "$DEPLOYER" 2>/dev/null || true)

if [[ -z "$DERIVED_ADMIN" ]]; then
  echo "ERROR: Could not derive a public address from DEPLOYER_SECRET." >&2
  echo "       Make sure DEPLOYER_SECRET is a valid Stellar secret key." >&2
  exit 1
fi

if [[ "$DERIVED_ADMIN" != "$ADMIN" ]]; then
  echo "ERROR: Keypair mismatch — DEPLOYER_SECRET does not match ADMIN_ADDRESS." >&2
  echo "       Derived : $DERIVED_ADMIN" >&2
  echo "       Expected: $ADMIN" >&2
  exit 1
fi
echo "    OK — signer matches admin address ($ADMIN)"

# ---------------------------------------------------------------------------
# Step 1: Snapshot current on-chain state
# ---------------------------------------------------------------------------
echo ""
echo "==> [1/5] Snapshotting on-chain state for $CONTRACT_NAME ($CONTRACT_ID)..."

OLD_VERSION=$(stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network "$NETWORK" \
  -- version 2>/dev/null || echo "unknown")
echo "    Current version : $OLD_VERSION"

# Capture instance state that must be re-applied after the WASM swap.
if [[ "$CONTRACT_NAME" == "scout_access" ]]; then
  echo "    Saving fee config (scout_access instance storage)..."
  FEE_CONFIG_JSON=$(stellar contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    -- get_fee_config 2>/dev/null || echo "")
  echo "    fee_config = $FEE_CONFIG_JSON"
fi

if [[ "$CONTRACT_NAME" == "verification" ]]; then
  echo "    NOTE: after upgrade, re-wire set_progress_contract if needed."
fi

# ---------------------------------------------------------------------------
# Step 2: Install new WASM and capture hash
# ---------------------------------------------------------------------------
echo ""
echo "==> [2/5] Installing new WASM: $WASM_PATH..."
NEW_WASM_HASH=$(stellar contract install \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  --wasm "$WASM_PATH")
echo "    New WASM hash: $NEW_WASM_HASH"

# ---------------------------------------------------------------------------
# Step 3: Call upgrade()
# ---------------------------------------------------------------------------
echo ""
echo "==> [3/5] Calling upgrade() on $CONTRACT_NAME..."
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- upgrade \
  --new_wasm_hash "$NEW_WASM_HASH"
echo "    Upgrade call succeeded."

# ---------------------------------------------------------------------------
# Step 4: Verify the contract is still healthy
# ---------------------------------------------------------------------------
echo ""
echo "==> [4/5] Verifying contract health..."
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network "$NETWORK" \
  -- health
echo "    Health check passed."

NEW_VERSION=$(stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network "$NETWORK" \
  -- version 2>/dev/null || echo "unknown")
echo "    New version : $NEW_VERSION"

# ---------------------------------------------------------------------------
# Step 5: Post-upgrade checklist
# ---------------------------------------------------------------------------
echo ""
echo "==> [5/5] Post-upgrade checklist"
echo ""
echo "  ✅ WASM replaced in-place — contract ID unchanged: $CONTRACT_ID"
echo "  ✅ Persistent storage (profiles, subscriptions, validators) untouched"
echo ""
echo "  ⚠  Instance storage is NOT automatically wiped but must be re-verified:"
echo ""

if [[ "$CONTRACT_NAME" == "scout_access" ]]; then
  echo "  scout_access — re-apply fee config if the storage layout changed:"
  echo ""
  echo "    stellar contract invoke --id $CONTRACT_ID \\"
  echo "      --source \$ADMIN_ADDRESS --network $NETWORK \\"
  echo "      -- update_fee_config --fee_config '$FEE_CONFIG_JSON'"
  echo ""
  echo "  scout_access — re-wire progress contract link if needed:"
  echo ""
  echo "    stellar contract invoke --id $CONTRACT_ID \\"
  echo "      --source \$ADMIN_ADDRESS --network $NETWORK \\"
  echo "      -- set_progress_contract --addr \$PROGRESS_CONTRACT_ID"
  echo ""
  echo "  Verify all cross-contract wiring:"
  echo "    ./scripts/verify-cross-contract-wiring.sh $NETWORK"
  echo ""
fi

if [[ "$CONTRACT_NAME" == "verification" ]]; then
  echo "  verification — re-wire progress contract link:"
  echo ""
  echo "    stellar contract invoke --id $CONTRACT_ID \\"
  echo "      --source \$ADMIN_ADDRESS --network $NETWORK \\"
  echo "      -- set_progress_contract --progress_contract \$PROGRESS_CONTRACT_ID"
  echo ""
  echo "  Verify all cross-contract wiring:"
  echo "    ./scripts/verify-cross-contract-wiring.sh $NETWORK"
  echo ""
fi

if [[ "$CONTRACT_NAME" == "progress" ]]; then
  echo "  progress — re-wire cross-contract links if needed:"
  echo ""
  echo "    stellar contract invoke --id $CONTRACT_ID \\"
  echo "      --source \$ADMIN_ADDRESS --network $NETWORK \\"
  echo "      -- set_verification_contract --addr \$VERIFICATION_CONTRACT_ID"
  echo ""
  echo "    stellar contract invoke --id $CONTRACT_ID \\"
  echo "      --source \$ADMIN_ADDRESS --network $NETWORK \\"
  echo "      -- set_registration_contract --addr \$REGISTRATION_CONTRACT_ID"
  echo ""
  echo "  Verify all cross-contract wiring:"
  echo "    ./scripts/verify-cross-contract-wiring.sh $NETWORK"
  echo ""
fi

echo "  Verify all cross-contract wiring:"
echo "    ./scripts/verify-cross-contract-wiring.sh $NETWORK"
echo ""
echo "  Regenerate TypeScript bindings if the ABI changed:"
echo "    ./scripts/generate-bindings.sh $NETWORK"
echo ""
echo "==> Upgrade complete: $CONTRACT_NAME $OLD_VERSION → $NEW_VERSION"
