#!/usr/bin/env bash
# ScoutChain — orchestrate a full "Address migration (new contract ID)".
#
# Use this ONLY when a bug cannot be fixed with an in-place upgrade()
# (e.g. a storage-layout change) and a fresh contract deploy with a NEW
# contract address is unavoidable. For a normal code-only fix, use
# scripts/upgrade.sh instead — that keeps the same contract ID and all state.
#
# This script chains the existing tooling, in the order documented in
# docs/DEPLOYMENT.md "Address migration (new contract ID)":
#
#   1. deploy.sh            — deploy the NEW contract set (snapshots the OLD ids)
#   2. initialize.sh        — initialize + wire the NEW contract set
#   3. pause old contracts  — pause_contract on each OLD id, so no new state is
#                             written to the addresses being retired
#   4. replay-state.sh      — replay the full supported state set onto NEW
#                             through migration-window-gated admin seeders
#   5. health-check.sh      — verify the NEW contract set is healthy
#   6. manual checklist     — bindings / backend / frontend / announce
#
# Usage:
#   ./scripts/migrate-contract.sh [network] [--dry-run] [--yes]
#
#   network     testnet | mainnet | local   (default: testnet)
#   --dry-run   Print every planned action WITHOUT executing any of them.
#   --yes, -y   Skip the interactive confirmation gates (for automation/CI).
#
# Requirements (same conventions as deploy.sh / initialize.sh):
#   • DEPLOYER_SECRET  — admin secret key (must match ADMIN_ADDRESS)
#   • ADMIN_ADDRESS, XLM_TOKEN_ADDRESS — required by initialize.sh
#   • A current .env.contracts describing the OLD (to-be-retired) contract set
#
# ===========================================================================
# MIGRATION REPLAY SCOPE
# ===========================================================================
#   replay-state.sh replays validators, profiles, progress history, milestones,
#   disputes, fee configuration, subscriptions, contacts, trial offers and
#   auto-renew flags. It opens the new contracts' migration windows only for
#   the duration of the replay and closes them before returning.
# ===========================================================================
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
NETWORK=""
DRY_RUN=0
ASSUME_YES=0

usage() {
  sed -n '2,44p' "$0" | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
  case "$arg" in
    --dry-run)     DRY_RUN=1 ;;
    --yes|-y)      ASSUME_YES=1 ;;
    -h|--help)     usage; exit 0 ;;
    testnet|mainnet|local) NETWORK="$arg" ;;
    *) echo "ERROR: unknown argument '$arg'" >&2; echo "Run '$0 --help' for usage." >&2; exit 1 ;;
  esac
done
NETWORK="${NETWORK:-testnet}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# read_id <file> <key> — print the value of KEY=... from an env file, or "".
read_id() {
  local file="$1" key="$2"
  [[ -f "$file" ]] || return 0
  grep -E "^${key}=" "$file" | head -1 | cut -d= -f2- || true
}

# confirm <prompt> — interactive y/N gate before a state-mutating step.
# Skippable with --yes. In --dry-run mode nothing mutates, so we don't prompt.
confirm() {
  local prompt="$1" reply
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "    [dry-run] confirmation gate: $prompt"
    return 0
  fi
  if [[ "$ASSUME_YES" -eq 1 ]]; then
    echo "    [--yes] auto-confirming: $prompt"
    return 0
  fi
  read -r -p "    $prompt [y/N] " reply
  case "$reply" in
    y|Y|yes|YES) return 0 ;;
    *) echo "==> Aborted by operator." >&2; exit 1 ;;
  esac
}

# run_step <description> <command...> — run (or, in dry-run, describe) a
# state-mutating command.
run_step() {
  local desc="$1"; shift
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "    [dry-run] would run ($desc): $*"
    return 0
  fi
  "$@"
}

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------
echo "=========================================================================="
echo "  ScoutChain ADDRESS MIGRATION (new contract ID) — network: $NETWORK"
echo "=========================================================================="
[[ "$DRY_RUN" -eq 1 ]] && echo "  Mode: DRY RUN — no contracts will be deployed, paused, or mutated."
echo ""

if [[ -z "${DEPLOYER_SECRET:-}" ]]; then
  echo "ERROR: DEPLOYER_SECRET is not set (admin secret key for deploy/init/pause/replay)." >&2
  exit 1
fi

# Capture the OLD (currently-live) contract IDs BEFORE deploy.sh overwrites
# .env.contracts. deploy.sh also snapshots .env.contracts -> .env.contracts.snapshot,
# but we hold the ids in variables so pausing does not depend on file timing.
OLD_REGISTRATION_CONTRACT_ID="$(read_id .env.contracts REGISTRATION_CONTRACT_ID)"
OLD_VERIFICATION_CONTRACT_ID="$(read_id .env.contracts VERIFICATION_CONTRACT_ID)"
OLD_PROGRESS_CONTRACT_ID="$(read_id .env.contracts PROGRESS_CONTRACT_ID)"
OLD_SCOUT_ACCESS_CONTRACT_ID="$(read_id .env.contracts SCOUT_ACCESS_CONTRACT_ID)"

if [[ -z "$OLD_REGISTRATION_CONTRACT_ID" ]]; then
  echo "ERROR: no existing .env.contracts found — nothing to migrate FROM." >&2
  echo "       A migration retires an existing contract set; deploy one first." >&2
  exit 1
fi

echo "  OLD (to be retired) contract set:"
echo "    registration : $OLD_REGISTRATION_CONTRACT_ID"
echo "    verification : $OLD_VERIFICATION_CONTRACT_ID"
echo "    progress     : $OLD_PROGRESS_CONTRACT_ID"
echo "    scout_access : $OLD_SCOUT_ACCESS_CONTRACT_ID"
echo ""

# Mainnet safety check (mirrors deploy.sh's FILL_IN_BEFORE_USE guard).
if [[ "$NETWORK" == "mainnet" ]]; then
  if grep -q "FILL_IN_BEFORE_USE" config/mainnet.json 2>/dev/null; then
    echo "ERROR: config/mainnet.json contains placeholder values (FILL_IN_BEFORE_USE)." >&2
    echo "       Update config/mainnet.json with real values before migrating on mainnet." >&2
    exit 1
  fi
  echo "  ⚠  MAINNET MIGRATION — this deploys NEW production contract addresses and"
  echo "     pauses the current ones. Double-check DEPLOYER_SECRET / ADMIN_ADDRESS."
  echo ""
fi

# ---------------------------------------------------------------------------
# Print the full plan up front (always), then gate.
# ---------------------------------------------------------------------------
cat <<PLAN
  Planned migration steps:
    [1/5] Deploy NEW contract set          -> scripts/deploy.sh $NETWORK
    [2/5] Initialize + wire NEW set        -> scripts/initialize.sh $NETWORK
    [3/5] Pause OLD contracts              -> pause_contract on each OLD id
    [4/5] Replay full supported state set  -> scripts/replay-state.sh $NETWORK
    [5/5] Health-check NEW set             -> scripts/health-check.sh $NETWORK
    Then: regenerate bindings, redeploy backend/frontend, announce old+new ids.

PLAN

confirm "Proceed with the address migration on '$NETWORK'?"

# ---------------------------------------------------------------------------
# [1/5] Deploy the NEW contract set
# ---------------------------------------------------------------------------
echo ""
echo "==> [1/5] Deploying NEW contract set..."
confirm "Deploy a brand-new contract set to '$NETWORK'?"
run_step "deploy" bash "$SCRIPT_DIR/deploy.sh" "$NETWORK"

# After deploy.sh, .env.contracts holds the NEW ids and .env.contracts.snapshot
# holds the OLD ids.
NEW_REGISTRATION_CONTRACT_ID="$(read_id .env.contracts REGISTRATION_CONTRACT_ID)"
NEW_VERIFICATION_CONTRACT_ID="$(read_id .env.contracts VERIFICATION_CONTRACT_ID)"
NEW_PROGRESS_CONTRACT_ID="$(read_id .env.contracts PROGRESS_CONTRACT_ID)"
NEW_SCOUT_ACCESS_CONTRACT_ID="$(read_id .env.contracts SCOUT_ACCESS_CONTRACT_ID)"

# ---------------------------------------------------------------------------
# [2/5] Initialize + wire the NEW contract set
# ---------------------------------------------------------------------------
echo ""
echo "==> [2/5] Initializing + wiring NEW contract set..."
confirm "Initialize and wire the new contract set?"
run_step "initialize" bash "$SCRIPT_DIR/initialize.sh" "$NETWORK"

# ---------------------------------------------------------------------------
# [3/5] Pause the OLD contracts so no new state is written to them
# ---------------------------------------------------------------------------
echo ""
echo "==> [3/5] Pausing OLD contracts..."
confirm "Pause the OLD contract set (${OLD_REGISTRATION_CONTRACT_ID} et al.)? This blocks all writes to the retired addresses."
for entry in \
  "registration:$OLD_REGISTRATION_CONTRACT_ID" \
  "verification:$OLD_VERIFICATION_CONTRACT_ID" \
  "progress:$OLD_PROGRESS_CONTRACT_ID" \
  "scout_access:$OLD_SCOUT_ACCESS_CONTRACT_ID"; do
  name="${entry%%:*}"
  id="${entry#*:}"
  [[ -z "$id" ]] && continue
  echo "    Pausing old $name ($id)..."
  run_step "pause $name" stellar contract invoke \
    --id "$id" \
    --source "$DEPLOYER_SECRET" \
    --network "$NETWORK" \
    -- pause_contract
done

# ---------------------------------------------------------------------------
# [4/5] Replay state: validators (automated) + player/scout export
# ---------------------------------------------------------------------------
echo ""
echo "==> [4/5] Replaying state (validators) + exporting players/scouts..."
REPLAY_FLAGS=()
[[ "$DRY_RUN" -eq 1 ]] && REPLAY_FLAGS+=(--dry-run)
[[ "$ASSUME_YES" -eq 1 ]] && REPLAY_FLAGS+=(--yes)

# Pass OLD/NEW ids explicitly (via env) so replay-state.sh does not depend on
# snapshot file timing. In dry-run, deploy did not run, so the NEW ids are still
# empty — fall back to the OLD ids for the call (replay-state.sh will not mutate
# anything in dry-run anyway). Computed into locals so the NEW_* vars used by
# the final summary below are left untouched.
REPLAY_NEW_REG="${NEW_REGISTRATION_CONTRACT_ID:-$OLD_REGISTRATION_CONTRACT_ID}"
REPLAY_NEW_VER="${NEW_VERIFICATION_CONTRACT_ID:-$OLD_VERIFICATION_CONTRACT_ID}"
REPLAY_NEW_PROGRESS="${NEW_PROGRESS_CONTRACT_ID:-$OLD_PROGRESS_CONTRACT_ID}"
REPLAY_NEW_SCOUT="${NEW_SCOUT_ACCESS_CONTRACT_ID:-$OLD_SCOUT_ACCESS_CONTRACT_ID}"
env \
  OLD_REGISTRATION_CONTRACT_ID="$OLD_REGISTRATION_CONTRACT_ID" \
  OLD_VERIFICATION_CONTRACT_ID="$OLD_VERIFICATION_CONTRACT_ID" \
  OLD_PROGRESS_CONTRACT_ID="$OLD_PROGRESS_CONTRACT_ID" \
  NEW_REGISTRATION_CONTRACT_ID="$REPLAY_NEW_REG" \
  NEW_VERIFICATION_CONTRACT_ID="$REPLAY_NEW_VER" \
  NEW_PROGRESS_CONTRACT_ID="$REPLAY_NEW_PROGRESS" \
  NEW_SCOUT_ACCESS_CONTRACT_ID="$REPLAY_NEW_SCOUT" \
  bash "$SCRIPT_DIR/replay-state.sh" "$NETWORK" "${REPLAY_FLAGS[@]}"

# ---------------------------------------------------------------------------
# [5/5] Health-check the NEW contract set
# ---------------------------------------------------------------------------
echo ""
echo "==> [5/5] Health-checking NEW contract set..."
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "    [dry-run] would run: scripts/health-check.sh $NETWORK"
else
  bash "$SCRIPT_DIR/health-check.sh" "$NETWORK"
fi

# ---------------------------------------------------------------------------
# Remaining manual steps
# ---------------------------------------------------------------------------
echo ""
echo "=========================================================================="
echo "  Migration orchestration complete — REMAINING MANUAL STEPS"
echo "=========================================================================="
echo "  .env.contracts now points at the NEW contract set:"
echo "    registration : ${NEW_REGISTRATION_CONTRACT_ID:-<see .env.contracts>}"
echo "    verification : ${NEW_VERIFICATION_CONTRACT_ID:-<see .env.contracts>}"
echo "    progress     : ${NEW_PROGRESS_CONTRACT_ID:-<see .env.contracts>}"
echo ""
echo "  1. Regenerate TypeScript bindings:"
echo "       ./scripts/generate-bindings.sh $NETWORK"
echo "  2. Redeploy backend + frontend with the new contract IDs."
echo "  3. Announce the migration in release notes with OLD and NEW ids:"
echo "       OLD registration: $OLD_REGISTRATION_CONTRACT_ID"
echo "       NEW registration: ${NEW_REGISTRATION_CONTRACT_ID:-<see .env.contracts>}"
echo ""
echo "  4. Replay exports are written under migration-export/ for audit and"
echo "        reconciliation. All supported state is seeded through the new"
echo "        contracts' migration-window-gated admin entrypoints."
echo ""
echo "  If anything looks wrong, the OLD contract set is only PAUSED (not"
echo "  deleted) and .env.contracts.snapshot still holds the old ids:"
echo "       ./scripts/rollback.sh $NETWORK"
echo "=========================================================================="
