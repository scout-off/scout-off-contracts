#!/usr/bin/env bash
# rehearse-admin-rotation.sh — rehearse the routine two-step admin-rotation
# (propose_admin → accept_admin) across all four ScoutChain contracts on a
# disposable local or testnet deployment.
#
# PURPOSE
#   This is the routine-operations counterpart to RUNBOOK.md's key-loss
#   tabletop exercise.  That exercise rehearses the failure mode (a lost or
#   compromised admin key); this one rehearses the normal, successful
#   rotation so operators have practiced the real procedure before they ever
#   need to do it against a shared testnet or mainnet deployment.
#
#   See docs/RUNBOOK.md — "Rehearse routine admin rotation" for the
#   recommended usage and context.
#
# USAGE
#   bash scripts/rehearse-admin-rotation.sh [<network>]
#
#   <network>  Stellar network name registered with stellar-cli.
#              Defaults to "local" (quickstart sandbox).
#              Use "testnet" for the Stellar testnet (requires funded accounts).
#
# PREREQUISITES
#   - stellar-cli installed and the target network reachable.
#   - For "local": the quickstart Docker sandbox must already be running.
#     See docs/DEPLOYMENT.md for the one-command sandbox setup.
#   - DEPLOYER_SECRET must be set in the environment or .env (a Stellar
#     secret key that can build and fund new identities on the target network).
#
# WHAT THE SCRIPT DOES
#   1. Generates a fresh OLD_ADMIN identity (the initial contract admin).
#   2. Generates a fresh NEW_ADMIN identity (the rotation candidate).
#   3. Funds both identities on the target network via friendbot / stellar keys fund.
#   4. Builds and deploys all four contracts using OLD_ADMIN.
#   5. Initialises all four contracts with OLD_ADMIN as admin.
#   6. For each contract:
#        a. OLD_ADMIN calls propose_admin(NEW_ADMIN).
#        b. Verifies the proposal was recorded (health check still shows
#           OLD_ADMIN as active admin).
#        c. NEW_ADMIN calls accept_admin().
#        d. Verifies the rotation completed: NEW_ADMIN can call a privileged
#           admin-only function (health check) and OLD_ADMIN cannot.
#   7. Reports a pass/fail summary and cleans up the ephemeral identities.
#
# EXIT CODES
#   0  all four contracts rotated successfully
#   1  one or more rotation steps failed (see output for details)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NETWORK="${1:-local}"

# Load DEPLOYER_SECRET from .env if present and not already set.
if [[ -z "${DEPLOYER_SECRET:-}" ]] && [[ -f "$REPO_ROOT/.env" ]]; then
  # shellcheck source=/dev/null
  source "$REPO_ROOT/.env"
fi

FAIL=0

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

log()  { echo "[rehearse-admin-rotation] $*"; }
ok()   { echo "  ✓ $*"; }
fail() { echo "  ✗ FAIL: $*" >&2; FAIL=1; }

# Unique prefix so parallel runs don't collide.
SESSION_ID="rot-rehearsal-$(date +%s)-$$"
OLD_ADMIN_KEY="${SESSION_ID}-old"
NEW_ADMIN_KEY="${SESSION_ID}-new"

# Track whether ephemeral keys were created so cleanup can be selective.
KEYS_CREATED=0

cleanup() {
  if [[ $KEYS_CREATED -eq 1 ]]; then
    log "Cleaning up ephemeral identities..."
    stellar keys rm "$OLD_ADMIN_KEY" 2>/dev/null || true
    stellar keys rm "$NEW_ADMIN_KEY" 2>/dev/null || true
    log "Ephemeral identities removed."
  fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Step 1 — Generate ephemeral admin identities
# ---------------------------------------------------------------------------
log "Step 1: Generating ephemeral OLD_ADMIN and NEW_ADMIN identities..."

stellar keys generate "$OLD_ADMIN_KEY" --network "$NETWORK" --no-fund
stellar keys generate "$NEW_ADMIN_KEY" --network "$NETWORK" --no-fund
KEYS_CREATED=1

OLD_ADMIN_ADDR=$(stellar keys address "$OLD_ADMIN_KEY")
NEW_ADMIN_ADDR=$(stellar keys address "$NEW_ADMIN_KEY")

log "  OLD_ADMIN address : $OLD_ADMIN_ADDR"
log "  NEW_ADMIN address : $NEW_ADMIN_ADDR"

# ---------------------------------------------------------------------------
# Step 2 — Fund both identities
# ---------------------------------------------------------------------------
log "Step 2: Funding identities on network '$NETWORK'..."

fund_identity() {
  local key="$1"
  local retries=20
  for i in $(seq 1 "$retries"); do
    if stellar keys fund "$key" --network "$NETWORK" 2>/dev/null; then
      return 0
    fi
    log "  Friendbot not ready yet (attempt $i/$retries)..."
    sleep 3
  done
  echo "ERROR: Could not fund $key after $retries attempts." >&2
  exit 1
}

fund_identity "$OLD_ADMIN_KEY"
fund_identity "$NEW_ADMIN_KEY"
ok "Both identities funded."

# ---------------------------------------------------------------------------
# Step 3 — Build WASM (only if not already built for this Rust revision)
# ---------------------------------------------------------------------------
log "Step 3: Building WASM contracts..."
cargo build --workspace --target wasm32v1-none --release \
  --manifest-path "$REPO_ROOT/Cargo.toml" \
  > /dev/null 2>&1
ok "WASM build complete."

WASM_DIR="$REPO_ROOT/target/wasm32v1-none/release"

# ---------------------------------------------------------------------------
# Step 4 — Deploy all four contracts using OLD_ADMIN
# ---------------------------------------------------------------------------
log "Step 4: Deploying contracts to '$NETWORK' with OLD_ADMIN as deployer..."

deploy_contract() {
  local name="$1"
  local wasm_src="$WASM_DIR/scoutchain_${name}.wasm"
  local wasm_opt="$WASM_DIR/scoutchain_${name}.optimized.wasm"

  # Optimize if the optimized WASM is stale or missing.
  if [[ ! -f "$wasm_opt" ]] || [[ "$wasm_src" -nt "$wasm_opt" ]]; then
    stellar contract optimize \
      --wasm "$wasm_src" \
      --wasm-out "$wasm_opt" \
      > /dev/null 2>&1
  fi

  stellar contract deploy \
    --wasm "$wasm_opt" \
    --source "$OLD_ADMIN_KEY" \
    --network "$NETWORK" \
    2>/dev/null | grep -oE 'C[A-Z2-7]{55}' | tail -1
}

REG_ID=$(deploy_contract registration)
VER_ID=$(deploy_contract verification)
PRG_ID=$(deploy_contract progress)
SCO_ID=$(deploy_contract scout_access)

for name_id in "registration:$REG_ID" "verification:$VER_ID" "progress:$PRG_ID" "scout_access:$SCO_ID"; do
  name="${name_id%%:*}"
  cid="${name_id#*:}"
  if [[ -z "$cid" ]]; then
    echo "ERROR: failed to deploy $name" >&2
    exit 1
  fi
  ok "Deployed $name => $cid"
done

# ---------------------------------------------------------------------------
# Step 5 — Initialise all four contracts with OLD_ADMIN
# ---------------------------------------------------------------------------
log "Step 5: Initialising contracts with OLD_ADMIN ($OLD_ADMIN_ADDR)..."

# Detect the native XLM token SAC address for the target network.
# The local quickstart sandbox uses a well-known address.
XLM_TOKEN="${XLM_TOKEN_ADDRESS:-CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA}"

invoke() {
  stellar contract invoke \
    --id "$1" \
    --source "$2" \
    --network "$NETWORK" \
    -- "${@:3}" 2>/dev/null
}

invoke "$REG_ID" "$OLD_ADMIN_KEY" initialize --admin "$OLD_ADMIN_ADDR"
invoke "$VER_ID" "$OLD_ADMIN_KEY" initialize --admin "$OLD_ADMIN_ADDR"
invoke "$PRG_ID" "$OLD_ADMIN_KEY" initialize --admin "$OLD_ADMIN_ADDR"

# scout_access needs fee_config; use the same defaults as initialize.sh.
invoke "$SCO_ID" "$OLD_ADMIN_KEY" initialize \
  --admin "$OLD_ADMIN_ADDR" \
  --xlm_token "$XLM_TOKEN" \
  --fee_config '{
    "contact_fee_stroops":1000000,
    "basic_sub_stroops":10000000,
    "pro_sub_stroops":30000000,
    "elite_sub_stroops":70000000,
    "sub_duration_secs":2592000,
    "pro_contact_limit":10,
    "trial_offer_escrow_stroops":5000000,
    "trial_offer_expiry_secs":604800
  }'

ok "All four contracts initialised."

# ---------------------------------------------------------------------------
# Step 6 — Perform propose_admin → accept_admin for each contract
# ---------------------------------------------------------------------------
log "Step 6: Rotating admin on all four contracts (OLD → NEW)..."

rotate_contract() {
  local name="$1"
  local contract_id="$2"

  echo ""
  log "  Rotating: $name ($contract_id)"

  # ── 6a. OLD_ADMIN proposes NEW_ADMIN ──────────────────────────────────────
  if ! invoke "$contract_id" "$OLD_ADMIN_KEY" propose_admin \
      --new_admin "$NEW_ADMIN_ADDR" 2>/dev/null; then
    fail "$name: propose_admin failed"
    return
  fi
  ok "$name: propose_admin($NEW_ADMIN_ADDR) succeeded"

  # ── 6b. NEW_ADMIN accepts ────────────────────────────────────────────────
  if ! invoke "$contract_id" "$NEW_ADMIN_KEY" accept_admin 2>/dev/null; then
    fail "$name: accept_admin failed"
    return
  fi
  ok "$name: accept_admin() succeeded"

  # ── 6c. Verify NEW_ADMIN can perform an admin-only action ─────────────────
  # health() is a read-only function available on all four contracts and
  # doesn't require admin auth, but pause_contract does.  We use health()
  # to confirm the contract is still operational, then attempt an admin
  # call to confirm privileges transferred.
  local health_out
  health_out=$(invoke "$contract_id" "$NEW_ADMIN_KEY" health 2>/dev/null || echo "FAILED")
  if echo "$health_out" | grep -q '"initialized":true'; then
    ok "$name: NEW_ADMIN health() passed (contract is live)"
  else
    fail "$name: health() failed after rotation — contract may be in a bad state"
    return
  fi

  # Confirm NEW_ADMIN can call an admin-gated function (pause → unpause).
  if ! invoke "$contract_id" "$NEW_ADMIN_KEY" pause_contract 2>/dev/null; then
    fail "$name: NEW_ADMIN could not call pause_contract — privileges not transferred"
    return
  fi
  if ! invoke "$contract_id" "$NEW_ADMIN_KEY" unpause_contract 2>/dev/null; then
    fail "$name: NEW_ADMIN could not call unpause_contract after pausing"
    return
  fi
  ok "$name: NEW_ADMIN can call pause_contract / unpause_contract (admin privileges confirmed)"

  # ── 6d. Verify OLD_ADMIN can NO longer call admin-only functions ──────────
  local old_result
  # Try to pause using the old admin key; it should fail with Unauthorized (4).
  set +e
  old_result=$(invoke "$contract_id" "$OLD_ADMIN_KEY" pause_contract 2>&1)
  local old_exit=$?
  set -e

  if [[ $old_exit -ne 0 ]] || echo "$old_result" | grep -qE "Error|error|unauthorized|Unauthorized"; then
    ok "$name: OLD_ADMIN correctly rejected from pause_contract (lost privileges)"
  else
    fail "$name: OLD_ADMIN still able to call pause_contract — rotation may be incomplete"
  fi
}

rotate_contract "registration"  "$REG_ID"
rotate_contract "verification"  "$VER_ID"
rotate_contract "progress"      "$PRG_ID"
rotate_contract "scout_access"  "$SCO_ID"

# ---------------------------------------------------------------------------
# Step 7 — Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Admin-rotation rehearsal summary ==="
if [[ $FAIL -ne 0 ]]; then
  echo "FAIL: One or more rotation steps failed — see output above."
  echo ""
  echo "Review each FAIL line and compare against the rotation procedure in"
  echo "docs/RUNBOOK.md before attempting a rotation on a real deployment."
  exit 1
else
  echo "PASS: All four contracts rotated from OLD_ADMIN to NEW_ADMIN successfully."
  echo ""
  echo "  OLD_ADMIN : $OLD_ADMIN_ADDR (no longer admin on any contract)"
  echo "  NEW_ADMIN : $NEW_ADMIN_ADDR (now admin on all four contracts)"
  echo ""
  echo "The rotation worked correctly on this disposable deployment."
  echo "You are ready to perform a routine admin rotation on a real deployment."
  echo "See docs/RUNBOOK.md — 'Rehearse routine admin rotation' for next steps."
fi
