#!/usr/bin/env bash
# ScoutChain — initialize all deployed contracts
# Run after deploy.sh. Requires .env.contracts to exist.
set -euo pipefail

source "$(dirname "$0")/network.sh" "${1:-}"
NETWORK="${1:-testnet}"
# shellcheck source=/dev/null
source .env.contracts

ADMIN="${ADMIN_ADDRESS:?Set ADMIN_ADDRESS}"
DEPLOYER="${DEPLOYER_SECRET:?Set DEPLOYER_SECRET}"

# Per-network parameters come from config/<network>.json; networks without a
# config file (e.g. local) fall back to the testnet values.
CONFIG_FILE="config/${NETWORK}.json"
[[ -f "$CONFIG_FILE" ]] || CONFIG_FILE="config/testnet.json"

# Reads the config, rejects missing/placeholder fields, enforces the bounds in
# docs/CONFIG_REFERENCE.md, and prints: <xlm_token> <reg_cooldown_secs> <fee_config_json>.
# XLM_TOKEN_ADDRESS in the environment overrides xlm_token_address (sandbox/local).
CONFIG_OUT=$(python3 - "$CONFIG_FILE" "${XLM_TOKEN_ADDRESS:-}" <<'PY'
import json, sys
path, token_env = sys.argv[1], sys.argv[2]
cfg = json.load(open(path))
def fail(msg):
    sys.exit(f"ERROR: {path}: {msg}")
def get(d, k):
    v = d.get(k)
    if v is None or (isinstance(v, str) and (not v or "FILL_IN_BEFORE_USE" in v)):
        fail(f"'{k}' is missing or still a placeholder")
    return v
token = token_env or get(cfg, "xlm_token_address")
cool = get(cfg, "reg_cooldown_secs")
if not isinstance(cool, int) or not 0 <= cool <= 604800:
    fail("reg_cooldown_secs must be an integer in 0..=604800")
fc = get(cfg, "fee_config")
for k in ("contact_fee_stroops", "basic_sub_stroops", "pro_sub_stroops", "elite_sub_stroops",
          "trial_offer_escrow_stroops"):
    try:
        fc[k] = int(get(fc, k))
    except (TypeError, ValueError):
        fail(f"fee_config.{k} must be an integer")
for k in ("sub_duration_secs", "trial_offer_expiry_secs", "pro_contact_limit"):
    if not isinstance(get(fc, k), int):
        fail(f"fee_config.{k} must be an integer")
def bound(k, lo, hi):
    if not lo <= fc[k] <= hi:
        fail(f"fee_config.{k}={fc[k]} outside {lo}..={hi}")
bound("contact_fee_stroops", 100_000, 100_000_000_000)
for k in ("basic_sub_stroops", "pro_sub_stroops", "elite_sub_stroops"):
    bound(k, 1_000_000, 1_000_000_000_000)
if not fc["basic_sub_stroops"] <= fc["pro_sub_stroops"] <= fc["elite_sub_stroops"]:
    fail("fee_config subscription prices must satisfy basic <= pro <= elite")
bound("sub_duration_secs", 1, 365 * 24 * 3600)
bound("pro_contact_limit", 1, 10_000)
if fc["trial_offer_escrow_stroops"] < 0 or (fc["trial_offer_escrow_stroops"] > 0 and fc["trial_offer_expiry_secs"] == 0):
    fail("fee_config trial offer escrow must be >= 0 and needs a non-zero expiry when > 0")
fc = {k: (str(v) if k.endswith("_stroops") else v) for k, v in fc.items()}
print(token, cool, json.dumps(fc, separators=(",", ":")))
PY
) || exit 1
read -r XLM_TOKEN REG_COOLDOWN_SECS FEE_CONFIG_JSON <<<"$CONFIG_OUT"

# ---------------------------------------------------------------------------
# Guard: verify that DEPLOYER_SECRET is the keypair for ADMIN_ADDRESS.
#
# If the signer and the admin address are different accounts the contracts
# will be initialized with an admin that no one controls, permanently locking
# every admin-gated operation (update_fee_config, withdraw_fees, pause, etc.).
# This check catches the most common mistake — using a throwaway test key in a
# production or shared-testnet deployment.
#
# stellar keys address <secret>  — prints the G-address derived from a secret
# key without any network call.  We compare it against $ADMIN_ADDRESS and abort
# before touching any contract if they differ.
# ---------------------------------------------------------------------------
echo "==> Verifying admin keypair..."
DERIVED_ADMIN=$(stellar keys address "$DEPLOYER" 2>/dev/null || true)

if [[ -z "$DERIVED_ADMIN" ]]; then
  echo "ERROR: Could not derive a public address from DEPLOYER_SECRET." >&2
  echo "       Make sure DEPLOYER_SECRET is a valid Stellar secret key (starts with 'S')." >&2
  exit 1
fi

if [[ "$DERIVED_ADMIN" != "$ADMIN" ]]; then
  echo "ERROR: Keypair mismatch — the signing key does not match ADMIN_ADDRESS." >&2
  echo "       Derived from DEPLOYER_SECRET : $DERIVED_ADMIN" >&2
  echo "       ADMIN_ADDRESS                : $ADMIN" >&2
  echo "" >&2
  echo "  Initializing with a mismatched admin permanently locks all admin operations." >&2
  echo "  Fix .env so that DEPLOYER_SECRET is the secret key for ADMIN_ADDRESS, then" >&2
  echo "  re-run this script." >&2
  exit 1
fi

echo "    OK — signer matches admin address ($ADMIN)"

# ---------------------------------------------------------------------------
# Run a `stellar contract invoke` command, treating a specific contract error
# code as "this step was already done" (skip, don't fail) rather than a fatal
# error. This lets the whole script be re-run safely after a partial failure
# instead of aborting on AlreadyInitialized / AlreadyConfigured.
# ---------------------------------------------------------------------------
invoke_idempotent() {
  local skip_error_code="$1"
  local description="$2"
  shift 2
  local output
  set +e
  output=$("$@" 2>&1)
  local status=$?
  set -e
  if [[ $status -ne 0 ]]; then
    if echo "$output" | grep -q "Error(Contract, #${skip_error_code})"; then
      echo "    $description already done — skipping"
      return 0
    fi
    echo "$output" >&2
    echo "ERROR: $description failed." >&2
    exit 1
  fi
  echo "$output"
}

echo "==> Initializing registration contract..."
invoke_idempotent 1 "registration initialize" \
  stellar contract invoke \
  --id "$REGISTRATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing verification contract..."
invoke_idempotent 1 "verification initialize" \
  stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing progress contract..."
invoke_idempotent 1 "progress initialize" \
  stellar contract invoke \
  --id "$PROGRESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN"

echo "==> Initializing scout_access contract..."
invoke_idempotent 1 "scout_access initialize" \
  stellar contract invoke \
  --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN" \
  --xlm_token "$XLM_TOKEN" \
  --fee_config "$FEE_CONFIG_JSON"

echo "==> Wiring verification → progress cross-contract link..."
# verification.set_progress_contract is first-time-only: it returns
# AlreadyConfigured (error 11) on every subsequent call so a stale address
# is never silently overwritten. That means a second run of this script
# (e.g. after a partial failure, or to re-wire a redeployed progress
# contract) would otherwise abort here. Detect that case and fall back to
# update_progress_contract so the script is idempotent.
set +e
SET_PROGRESS_CONTRACT_OUTPUT=$(stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --progress_contract "$PROGRESS_CONTRACT_ID" 2>&1)
SET_PROGRESS_CONTRACT_STATUS=$?
set -e

if [[ $SET_PROGRESS_CONTRACT_STATUS -ne 0 ]]; then
  if echo "$SET_PROGRESS_CONTRACT_OUTPUT" | grep -q "Error(Contract, #11)"; then
    echo "    verification progress contract already configured — re-wiring via update_progress_contract"
    stellar contract invoke \
      --id "$VERIFICATION_CONTRACT_ID" \
      --source "$DEPLOYER" \
      --network "$NETWORK" \
      -- update_progress_contract \
      --progress_contract "$PROGRESS_CONTRACT_ID"
  else
    echo "$SET_PROGRESS_CONTRACT_OUTPUT" >&2
    echo "ERROR: set_progress_contract failed on the verification contract." >&2
    exit 1
  fi
else
  echo "$SET_PROGRESS_CONTRACT_OUTPUT"
fi

echo "==> Wiring registration ← progress cross-contract link..."
stellar contract invoke \
  --id "$REGISTRATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --addr "$PROGRESS_CONTRACT_ID"

echo "==> Wiring verification → registration cross-contract link..."
# verification.set_registration_contract carries the same first-call-only
# guard as set_progress_contract above (AlreadyConfigured, error #11) — same
# idempotent fallback to update_registration_contract on a re-run.
set +e
SET_REG_CONTRACT_OUTPUT=$(stellar contract invoke \
  --id "$VERIFICATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_registration_contract \
  --reg_contract "$REGISTRATION_CONTRACT_ID" 2>&1)
SET_REG_CONTRACT_STATUS=$?
set -e

if [[ $SET_REG_CONTRACT_STATUS -ne 0 ]]; then
  if echo "$SET_REG_CONTRACT_OUTPUT" | grep -q "Error(Contract, #11)"; then
    echo "    verification registration contract already configured — re-wiring via update_registration_contract"
    stellar contract invoke \
      --id "$VERIFICATION_CONTRACT_ID" \
      --source "$DEPLOYER" \
      --network "$NETWORK" \
      -- update_registration_contract \
      --reg_contract "$REGISTRATION_CONTRACT_ID"
  else
    echo "$SET_REG_CONTRACT_OUTPUT" >&2
    echo "ERROR: set_registration_contract failed on the verification contract." >&2
    exit 1
  fi
else
  echo "$SET_REG_CONTRACT_OUTPUT"
fi

echo "==> Wiring progress → verification cross-contract link..."
stellar contract invoke \
  --id "$PROGRESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_verification_contract \
  --addr "$VERIFICATION_CONTRACT_ID"

echo "==> Wiring progress → registration cross-contract link..."
stellar contract invoke \
  --id "$PROGRESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_registration_contract \
  --addr "$REGISTRATION_CONTRACT_ID"

echo "==> Wiring progress → scout_access cross-contract link..."
stellar contract invoke \
  --id "$PROGRESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_scout_access_contract \
  --addr "$SCOUT_ACCESS_CONTRACT_ID"

echo "==> Wiring scout_access → progress cross-contract link..."
stellar contract invoke \
  --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_progress_contract \
  --addr "$PROGRESS_CONTRACT_ID"

echo "==> Wiring scout_access → registration cross-contract link..."
stellar contract invoke \
  --id "$SCOUT_ACCESS_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_registration_contract \
  --addr "$REGISTRATION_CONTRACT_ID"

echo ""
echo "==> Setting registration cooldown..."
# Value comes from config/<network>.json (validated above).
stellar contract invoke \
  --id "$REGISTRATION_CONTRACT_ID" \
  --source "$DEPLOYER" \
  --network "$NETWORK" \
  -- set_reg_cooldown \
  --cooldown_secs "$REG_COOLDOWN_SECS"

echo ""
echo "==> Verifying wiring consistency (post-wiring gate)..."
# Every wiring call above is a separate, independently-failable
# `stellar contract invoke` — Soroban has no atomic multi-contract
# transaction primitive, so this script cannot guarantee the calls above
# landed consistently just because none of them returned a non-zero exit
# code individually (e.g. a call could succeed while wiring the wrong
# address due to an env var mistake). Gate success on an actual
# cross-contract consistency check rather than assuming it from the absence
# of errors above. See docs/WIRING_REGISTRY_DESIGN.md and issue #1041.
if ! bash "$(dirname "${BASH_SOURCE[0]}")/verify-cross-contract-wiring.sh" "$NETWORK"; then
  echo "" >&2
  echo "ERROR: post-wiring consistency check failed — see output above." >&2
  echo "       Run scripts/verify-cross-contract-wiring.sh --repair for the exact corrective calls." >&2
  exit 1
fi

echo ""
echo "==> Querying deployed contract versions..."
for entry in \
  "registration:$REGISTRATION_CONTRACT_ID" \
  "verification:$VERIFICATION_CONTRACT_ID" \
  "progress:$PROGRESS_CONTRACT_ID" \
  "scout_access:$SCOUT_ACCESS_CONTRACT_ID"; do
  name="${entry%%:*}"
  id="${entry#*:}"
  version=$(stellar contract invoke \
    --id "$id" \
    --source "$DEPLOYER" \
    --network "$NETWORK" \
    -- version)
  echo "    $name version => $version"
done

echo ""
echo "==> All contracts initialized and wired."
