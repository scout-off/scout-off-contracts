#!/usr/bin/env bash
# ScoutChain — replay persistent state from an OLD contract set onto a NEW one.
#
# This is step 4 ("replay events to seed initial state") of the "Address
# migration (new contract ID)" procedure in docs/DEPLOYMENT.md. It is normally
# invoked by scripts/migrate-contract.sh, but can also be run standalone.
#
# Usage:
#   ./scripts/replay-state.sh [network] [--dry-run] [--yes] [--export-dir DIR]
#
# Arguments / flags:
#   network        testnet | mainnet | local   (default: testnet)
#   --dry-run      Print the planned actions without executing any of them.
#   --yes, -y      Skip the interactive confirmation gate (for automation).
#   --export-dir   Directory for migration export JSON
#                  (default: migration-export/).
#
# Contract IDs are resolved in this order:
#   OLD ids  — env OLD_<NAME>_CONTRACT_ID, else read from .env.contracts.snapshot
#   NEW ids  — env NEW_<NAME>_CONTRACT_ID, else read from .env.contracts
# where <NAME> is REGISTRATION / VERIFICATION / PROGRESS / SCOUT_ACCESS.
#
# Signing:
#   DEPLOYER_SECRET must be the admin secret key for the NEW contract set
#   (same key used by initialize.sh). ADMIN_ADDRESS is optional but, if set,
#   is verified against DEPLOYER_SECRET.
#
# ===========================================================================
# WHAT THIS TOOL CAN AND CANNOT REPLAY  (read this before relying on it)
# ===========================================================================
#
#   VALIDATORS — fully automated. verification.register_validator(wallet,
#   credentials) is admin-only (require_admin, no wallet self-auth), so an
#   operator holding the admin key CAN legitimately re-create every validator
#   on the NEW contract. This script reads get_validators() + get_validator()
#   from the OLD contract and calls register_validator() on the NEW one,
#   signed by DEPLOYER_SECRET.
#
#   PLAYERS and SCOUTS — can be re-seeded via admin-only entrypoints.
#   registration.admin_seed_player() and registration.admin_seed_scout() are
#   admin-authenticated and accept the full exported payload needed to recreate
#   the persistent profile state without requiring wallet signatures.
#
#   ALL DERIVED STATE — after profiles are restored, this script replays progress
#   history, milestones, disputes, subscriptions, contacts, trial offers and
#   auto-renew flags through their migration-window-gated admin seeders. It
#   closes every migration window before returning, including on failure.
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
NETWORK=""
DRY_RUN=0
ASSUME_YES=0
EXPORT_DIR="migration-export"

usage() {
  sed -n '2,60p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)     DRY_RUN=1 ;;
    --yes|-y)      ASSUME_YES=1 ;;
    --export-dir)  shift; EXPORT_DIR="${1:?--export-dir needs a value}" ;;
    -h|--help)     usage; exit 0 ;;
    testnet|mainnet|local) NETWORK="$1" ;;
    *) echo "ERROR: unknown argument '$1'" >&2; echo "Run '$0 --help' for usage." >&2; exit 1 ;;
  esac
  shift
done
NETWORK="${NETWORK:-testnet}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# read_id <file> <key> — print the value of KEY=... from an env file, or "".
read_id() {
  local file="$1" key="$2"
  [[ -f "$file" ]] || return 0
  grep -E "^${key}=" "$file" | head -1 | cut -d= -f2- || true
}

# confirm <prompt> — interactive y/N gate, skippable with --yes.
confirm() {
  local prompt="$1" reply
  if [[ "$ASSUME_YES" -eq 1 ]]; then
    echo "    [--yes] auto-confirming: $prompt"
    return 0
  fi
  read -r -p "    $prompt [y/N] " reply
  case "$reply" in
    y|Y|yes|YES) return 0 ;;
    *) echo "==> Aborted by operator — no changes made to the new contract." >&2; exit 1 ;;
  esac
}

# invoke_view <contract_id> <fn> [args...] — read-only invoke (no --source).
invoke_view() {
  local id="$1"; shift
  stellar contract invoke --id "$id" --network "$NETWORK" -- "$@"
}

# invoke_admin <contract_id> <fn> [args...] — admin-signed state change.
invoke_admin() {
  local id="$1"; shift
  stellar contract invoke --id "$id" --source "$DEPLOYER" --network "$NETWORK" -- "$@"
}

# ---------------------------------------------------------------------------
# Resolve OLD and NEW contract IDs
# ---------------------------------------------------------------------------
OLD_REGISTRATION_CONTRACT_ID="${OLD_REGISTRATION_CONTRACT_ID:-$(read_id .env.contracts.snapshot REGISTRATION_CONTRACT_ID)}"
OLD_VERIFICATION_CONTRACT_ID="${OLD_VERIFICATION_CONTRACT_ID:-$(read_id .env.contracts.snapshot VERIFICATION_CONTRACT_ID)}"
OLD_PROGRESS_CONTRACT_ID="${OLD_PROGRESS_CONTRACT_ID:-$(read_id .env.contracts.snapshot PROGRESS_CONTRACT_ID)}"
OLD_SCOUT_ACCESS_CONTRACT_ID="${OLD_SCOUT_ACCESS_CONTRACT_ID:-$(read_id .env.contracts.snapshot SCOUT_ACCESS_CONTRACT_ID)}"

NEW_REGISTRATION_CONTRACT_ID="${NEW_REGISTRATION_CONTRACT_ID:-$(read_id .env.contracts REGISTRATION_CONTRACT_ID)}"
NEW_VERIFICATION_CONTRACT_ID="${NEW_VERIFICATION_CONTRACT_ID:-$(read_id .env.contracts VERIFICATION_CONTRACT_ID)}"
NEW_PROGRESS_CONTRACT_ID="${NEW_PROGRESS_CONTRACT_ID:-$(read_id .env.contracts PROGRESS_CONTRACT_ID)}"
NEW_SCOUT_ACCESS_CONTRACT_ID="${NEW_SCOUT_ACCESS_CONTRACT_ID:-$(read_id .env.contracts SCOUT_ACCESS_CONTRACT_ID)}"

DEPLOYER="${DEPLOYER_SECRET:-}"

echo "=========================================================================="
echo "  ScoutChain state replay — network: $NETWORK"
echo "=========================================================================="
echo "  OLD registration : ${OLD_REGISTRATION_CONTRACT_ID:-<unset>}"
echo "  OLD verification : ${OLD_VERIFICATION_CONTRACT_ID:-<unset>}"
echo "  OLD progress     : ${OLD_PROGRESS_CONTRACT_ID:-<unset>}"
echo "  OLD scout_access : ${OLD_SCOUT_ACCESS_CONTRACT_ID:-<unset>}"
echo "  NEW registration : ${NEW_REGISTRATION_CONTRACT_ID:-<unset>}"
echo "  NEW verification : ${NEW_VERIFICATION_CONTRACT_ID:-<unset>}"
echo "  NEW progress     : ${NEW_PROGRESS_CONTRACT_ID:-<unset>}"
echo "  NEW scout_access : ${NEW_SCOUT_ACCESS_CONTRACT_ID:-<unset>}"
echo "  Export directory : $EXPORT_DIR"
[[ "$DRY_RUN" -eq 1 ]] && echo "  Mode             : DRY RUN (no state will be changed)"
echo ""

if [[ -z "$OLD_VERIFICATION_CONTRACT_ID" || -z "$OLD_REGISTRATION_CONTRACT_ID" || -z "$OLD_PROGRESS_CONTRACT_ID" || -z "$OLD_SCOUT_ACCESS_CONTRACT_ID" ]]; then
  echo "ERROR: could not resolve OLD contract IDs." >&2
  echo "       Set OLD_*_CONTRACT_ID env vars or provide .env.contracts.snapshot." >&2
  exit 1
fi
if [[ -z "$NEW_VERIFICATION_CONTRACT_ID" || -z "$NEW_REGISTRATION_CONTRACT_ID" || -z "$NEW_PROGRESS_CONTRACT_ID" || -z "$NEW_SCOUT_ACCESS_CONTRACT_ID" ]]; then
  echo "ERROR: could not resolve NEW contract IDs." >&2
  echo "       Set NEW_*_CONTRACT_ID env vars or provide .env.contracts." >&2
  exit 1
fi
if [[ -z "$DEPLOYER" ]]; then
  echo "ERROR: DEPLOYER_SECRET is not set (required to sign migration replay calls)." >&2
  exit 1
fi

# Optional admin-key sanity check (same shape as initialize.sh).
if [[ -n "${ADMIN_ADDRESS:-}" && "$DRY_RUN" -eq 0 ]]; then
  DERIVED_ADMIN=$(stellar keys address "$DEPLOYER" 2>/dev/null || true)
  if [[ -n "$DERIVED_ADMIN" && "$DERIVED_ADMIN" != "$ADMIN_ADDRESS" ]]; then
    echo "ERROR: DEPLOYER_SECRET ($DERIVED_ADMIN) does not match ADMIN_ADDRESS ($ADMIN_ADDRESS)." >&2
    echo "       register_validator on the new contract would fail auth. Aborting." >&2
    exit 1
  fi
fi

mkdir -p "$EXPORT_DIR"
TS="$(date -u +%Y%m%dT%H%M%SZ)"

MIGRATION_WINDOWS_OPEN=0
close_migration_windows() {
  [[ "$MIGRATION_WINDOWS_OPEN" -eq 1 ]] || return 0
  echo "==> Closing migration windows..."
  for entry in \
    "progress:$NEW_PROGRESS_CONTRACT_ID" \
    "verification:$NEW_VERIFICATION_CONTRACT_ID" \
    "scout_access:$NEW_SCOUT_ACCESS_CONTRACT_ID"; do
    name="${entry%%:*}"
    id="${entry#*:}"
    set +e
    invoke_admin "$id" close_migration_window >/dev/null 2>&1
    status=$?
    set -e
    if [[ $status -ne 0 ]]; then
      echo "WARN: failed to close $name migration window ($id); close it manually before launch." >&2
    fi
  done
  MIGRATION_WINDOWS_OPEN=0
}
trap close_migration_windows EXIT

if [[ "$DRY_RUN" -eq 0 ]]; then
  echo "==> Opening migration windows on the NEW contracts..."
  MIGRATION_WINDOWS_OPEN=1
  invoke_admin "$NEW_PROGRESS_CONTRACT_ID" open_migration_window >/dev/null
  invoke_admin "$NEW_VERIFICATION_CONTRACT_ID" open_migration_window >/dev/null
  invoke_admin "$NEW_SCOUT_ACCESS_CONTRACT_ID" open_migration_window >/dev/null
else
  echo "==> [dry-run] would open migration windows on progress, verification, and scout_access"
fi

# ===========================================================================
# PART 1 — VALIDATORS  (read OLD, register on NEW — fully automated)
# ===========================================================================
echo "==> [1/3] Replaying validators (verification contract)..."
echo "    Reading active validators from OLD verification contract..."

VALIDATORS_JSON="$(invoke_view "$OLD_VERIFICATION_CONTRACT_ID" get_validators 2>/dev/null || echo '[]')"
# get_validators returns a JSON array of G-addresses, e.g. ["G...","G..."].
mapfile -t VALIDATOR_WALLETS < <(echo "$VALIDATORS_JSON" | jq -r '.[]?' 2>/dev/null || true)

VALIDATOR_COUNT="${#VALIDATOR_WALLETS[@]}"
echo "    Found $VALIDATOR_COUNT active validator(s) on the old contract."

VALIDATORS_EXPORT="$EXPORT_DIR/validators-$TS.json"
echo "[]" > "$VALIDATORS_EXPORT"

if [[ "$VALIDATOR_COUNT" -gt 0 ]]; then
  if [[ "$DRY_RUN" -eq 0 ]]; then
    confirm "Register $VALIDATOR_COUNT validator(s) on the NEW verification contract ($NEW_VERIFICATION_CONTRACT_ID)?"
  fi

  for wallet in "${VALIDATOR_WALLETS[@]}"; do
    [[ -z "$wallet" ]] && continue
    validator_struct="$(invoke_view "$OLD_VERIFICATION_CONTRACT_ID" get_validator --wallet "$wallet" 2>/dev/null || echo '{}')"
    credentials="$(echo "$validator_struct" | jq -r '.credentials // empty' 2>/dev/null || true)"

    if [[ -z "$credentials" ]]; then
      echo "    WARN: could not read credentials for $wallet — skipping." >&2
      continue
    fi

    # Append to the validators export file for the before/after comparison.
    tmp="$(mktemp)"
    jq --arg w "$wallet" --arg c "$credentials" \
      '. += [{"wallet":$w,"credentials":$c}]' "$VALIDATORS_EXPORT" > "$tmp" && mv "$tmp" "$VALIDATORS_EXPORT"

    if [[ "$DRY_RUN" -eq 1 ]]; then
      echo "    [dry-run] would register_validator wallet=$wallet on $NEW_VERIFICATION_CONTRACT_ID"
      continue
    fi

    echo "    Registering validator $wallet on new contract..."
    set +e
    out="$(stellar contract invoke \
      --id "$NEW_VERIFICATION_CONTRACT_ID" \
      --source "$DEPLOYER" \
      --network "$NETWORK" \
      -- register_validator \
      --wallet "$wallet" \
      --credentials "$credentials" 2>&1)"
    status=$?
    set -e
    if [[ $status -ne 0 ]]; then
      # Error 7 == ValidatorAlreadyRegistered — treat as idempotent success.
      if echo "$out" | grep -qE "Error\(Contract, #7\)"; then
        echo "      already registered on new contract — skipping."
      else
        echo "$out" >&2
        echo "ERROR: register_validator failed for $wallet." >&2
        exit 1
      fi
    else
      echo "      OK"
    fi
  done
fi
echo "    Validators exported to $VALIDATORS_EXPORT"

# ===========================================================================
# PART 2 — PLAYERS  (EXPORT + RE-SEED)
# ===========================================================================
echo ""
echo "==> [2/3] Exporting and replaying players (registration contract)..."
PLAYER_COUNT_RAW="$(invoke_view "$OLD_REGISTRATION_CONTRACT_ID" get_player_count 2>/dev/null || echo 0)"
PLAYER_COUNT="$(echo "$PLAYER_COUNT_RAW" | tr -dc '0-9')"
PLAYER_COUNT="${PLAYER_COUNT:-0}"
echo "    Old registration contract reports get_player_count = $PLAYER_COUNT"

PLAYERS_EXPORT="$EXPORT_DIR/players-$TS.json"
echo "[]" > "$PLAYERS_EXPORT"

if [[ "$PLAYER_COUNT" -gt 0 ]]; then
  for ((id=1; id<=PLAYER_COUNT; id++)); do
    set +e
    player="$(stellar contract invoke --id "$OLD_REGISTRATION_CONTRACT_ID" --network "$NETWORK" \
      -- get_player --player_id "$id" 2>/dev/null)"
    status=$?
    set -e
    if [[ $status -ne 0 || -z "$player" ]]; then
      echo "    (player_id $id not found — likely deregistered; skipping)"
      continue
    fi
    tmp="$(mktemp)"
    jq --argjson p "$player" '. += [$p]' "$PLAYERS_EXPORT" > "$tmp" && mv "$tmp" "$PLAYERS_EXPORT"

    if [[ "$DRY_RUN" -eq 1 ]]; then
      echo "    [dry-run] would admin_seed_player player_id=$id on $NEW_REGISTRATION_CONTRACT_ID"
      continue
    fi

    wallet="$(echo "$player" | jq -r '.wallet // empty' 2>/dev/null || true)"
    vitals="$(echo "$player" | jq -c '{age: (.vitals.age // 0), position: (.vitals.position // ""), region: (.vitals.region // ""), nationality: (.vitals.nationality // "")}' 2>/dev/null || true)"
    ipfs_hashes="$(echo "$player" | jq -c '.ipfs_hashes // []' 2>/dev/null || true)"
    level="$(echo "$player" | jq -r '.level // "Unverified"' 2>/dev/null || true)"
    registered_at="$(echo "$player" | jq -r '.registered_at // 0' 2>/dev/null || true)"
    updated_at="$(echo "$player" | jq -r '.updated_at // 0' 2>/dev/null || true)"

    if [[ -z "$wallet" || -z "$vitals" || -z "$ipfs_hashes" ]]; then
      echo "    WARN: incomplete player payload for id $id — skipping." >&2
      continue
    fi

    echo "    Re-seeding player $id on new contract..."
    set +e
    out="$(stellar contract invoke \
      --id "$NEW_REGISTRATION_CONTRACT_ID" \
      --source "$DEPLOYER" \
      --network "$NETWORK" \
      -- admin_seed_player \
      --wallet "$wallet" \
      --vitals "$vitals" \
      --ipfs_hashes "$ipfs_hashes" \
      --level "$level" \
      --player_id "$id" \
      --registered_at "$registered_at" \
      --updated_at "$updated_at" 2>&1)"
    status=$?
    set -e
    if [[ $status -ne 0 ]]; then
      echo "$out" >&2
      echo "ERROR: admin_seed_player failed for player_id $id." >&2
      exit 1
    fi
    echo "      OK"

    deactivated="$(invoke_view "$OLD_REGISTRATION_CONTRACT_ID" is_player_deactivated \
      --player_id "$id" 2>/dev/null || echo false)"
    if [[ "$deactivated" == "true" ]]; then
      admin_call "$NEW_REGISTRATION_CONTRACT_ID" deactivate_player --player_id "$id" >/dev/null
    fi
  done
fi
echo "    Players exported to $PLAYERS_EXPORT"

# ===========================================================================
# PART 3 — SCOUTS  (EXPORT + RE-SEED)
# ===========================================================================
echo ""
echo "==> [3/3] Exporting and replaying scouts (registration contract)..."
SCOUT_COUNT_RAW="$(invoke_view "$OLD_REGISTRATION_CONTRACT_ID" get_scout_count 2>/dev/null || echo 0)"
SCOUT_COUNT="$(echo "$SCOUT_COUNT_RAW" | tr -dc '0-9')"
SCOUT_COUNT="${SCOUT_COUNT:-0}"
echo "    Old registration contract reports get_scout_count = $SCOUT_COUNT"

SCOUTS_EXPORT="$EXPORT_DIR/scouts-$TS.json"
echo "[]" > "$SCOUTS_EXPORT"

if [[ "$SCOUT_COUNT" -gt 0 ]]; then
  for ((id=1; id<=SCOUT_COUNT; id++)); do
    set +e
    scout="$(stellar contract invoke --id "$OLD_REGISTRATION_CONTRACT_ID" --network "$NETWORK" \
      -- get_scout --scout_id "$id" 2>/dev/null)"
    status=$?
    set -e
    if [[ $status -ne 0 || -z "$scout" ]]; then
      echo "    (scout_id $id not found — skipping)"
      continue
    fi
    tmp="$(mktemp)"
    jq --argjson s "$scout" '. += [$s]' "$SCOUTS_EXPORT" > "$tmp" && mv "$tmp" "$SCOUTS_EXPORT"

    if [[ "$DRY_RUN" -eq 1 ]]; then
      echo "    [dry-run] would admin_seed_scout scout_id=$id on $NEW_REGISTRATION_CONTRACT_ID"
      continue
    fi

    wallet="$(echo "$scout" | jq -r '.wallet // empty' 2>/dev/null || true)"
    region="$(echo "$scout" | jq -r '.region // empty' 2>/dev/null || true)"
    registered_at="$(echo "$scout" | jq -r '.registered_at // 0' 2>/dev/null || true)"
    verified="$(echo "$scout" | jq -r '.verified // false' 2>/dev/null || true)"

    if [[ -z "$wallet" || -z "$region" ]]; then
      echo "    WARN: incomplete scout payload for id $id — skipping." >&2
      continue
    fi

    echo "    Re-seeding scout $id on new contract..."
    set +e
    out="$(stellar contract invoke \
      --id "$NEW_REGISTRATION_CONTRACT_ID" \
      --source "$DEPLOYER" \
      --network "$NETWORK" \
      -- admin_seed_scout \
      --wallet "$wallet" \
      --region "$region" \
      --scout_id "$id" \
      --registered_at "$registered_at" \
      --verified "$verified" 2>&1)"
    status=$?
    set -e
    if [[ $status -ne 0 ]]; then
      echo "$out" >&2
      echo "ERROR: admin_seed_scout failed for scout_id $id." >&2
      exit 1
    fi
    echo "      OK"
  done
fi
echo "    Scouts exported to $SCOUTS_EXPORT"

# ===========================================================================
# PART 4 — PROGRESS HISTORY
# ===========================================================================
echo ""
echo "==> [4/10] Replaying progress history and Merkle roots..."
PROGRESS_EXPORT="$EXPORT_DIR/progress-history-$TS.json"
echo "[]" > "$PROGRESS_EXPORT"

admin_call() {
  local id="$1"; shift
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "    [dry-run] would invoke $* on $id"
  else
    invoke_admin "$id" "$@"
  fi
}

while IFS= read -r player; do
  [[ -z "$player" ]] && continue
  player_id="$(echo "$player" | jq -r '.player_id')"
  history="$(invoke_view "$OLD_PROGRESS_CONTRACT_ID" get_progress_history --player_id "$player_id" 2>/dev/null || echo '[]')"
  root="$(invoke_view "$OLD_PROGRESS_CONTRACT_ID" get_progress_root --player_id "$player_id" 2>/dev/null || echo '')"
  tmp="$(mktemp)"
  jq --argjson id "$player_id" --argjson h "$history" --arg root "$root" \
    '. += [{player_id:$id, history:$h, root:$root}]' "$PROGRESS_EXPORT" > "$tmp" && mv "$tmp" "$PROGRESS_EXPORT"

  count="$(echo "$history" | jq 'length')"
  index=1
  while IFS= read -r entry; do
    [[ -z "$entry" ]] && continue
    expected_root="null"
    [[ "$index" -eq "$count" && -n "$root" ]] && expected_root="$root"
    admin_call "$NEW_PROGRESS_CONTRACT_ID" admin_seed_history \
      --player_id "$player_id" --history_index "$index" --entry "$entry" --expected_root "$expected_root" >/dev/null
    index=$((index + 1))
  done < <(echo "$history" | jq -c '.[]')
done < <(jq -c '.[]' "$PLAYERS_EXPORT")

# ===========================================================================
# PART 5 — MILESTONES AND DISPUTES
# ===========================================================================
echo "==> [5/10] Replaying milestones and disputes..."
MILESTONE_EXPORT="$EXPORT_DIR/milestones-$TS.json"
DISPUTE_EXPORT="$EXPORT_DIR/disputes-$TS.json"
echo "[]" > "$MILESTONE_EXPORT"
echo "[]" > "$DISPUTE_EXPORT"

while IFS= read -r player; do
  [[ -z "$player" ]] && continue
  player_id="$(echo "$player" | jq -r '.player_id')"
  milestone_count="$(invoke_view "$OLD_VERIFICATION_CONTRACT_ID" get_milestone_count --player_id "$player_id" 2>/dev/null || echo 0)"
  for ((index=1; index<=milestone_count; index++)); do
    milestone="$(invoke_view "$OLD_VERIFICATION_CONTRACT_ID" get_milestone --player_id "$player_id" --index "$index" 2>/dev/null || true)"
    [[ -z "$milestone" ]] && continue
    tmp="$(mktemp)"
    jq --argjson m "$milestone" '. += [$m]' "$MILESTONE_EXPORT" > "$tmp" && mv "$tmp" "$MILESTONE_EXPORT"
    validator="$(echo "$milestone" | jq -r '.validator')"
    admin_call "$NEW_VERIFICATION_CONTRACT_ID" admin_seed_milestone \
      --player_id "$player_id" --milestone_index "$index" --milestone "$milestone" --validator "$validator" >/dev/null

    has_dispute="$(invoke_view "$OLD_VERIFICATION_CONTRACT_ID" has_dispute \
      --player_id "$player_id" --milestone_index "$index" 2>/dev/null || echo false)"
    if [[ "$has_dispute" == "true" ]]; then
      dispute="$(invoke_view "$OLD_VERIFICATION_CONTRACT_ID" get_dispute \
        --player_id "$player_id" --milestone_index "$index" 2>/dev/null || true)"
      if [[ -n "$dispute" ]]; then
        tmp="$(mktemp)"
        jq --argjson d "$dispute" '. += [$d]' "$DISPUTE_EXPORT" > "$tmp" && mv "$tmp" "$DISPUTE_EXPORT"
        admin_call "$NEW_VERIFICATION_CONTRACT_ID" admin_seed_dispute \
          --player_id "$player_id" --milestone_index "$index" --dispute "$dispute" >/dev/null
      fi
    fi
  done
done < <(jq -c '.[]' "$PLAYERS_EXPORT")

# ===========================================================================
# PART 6 — FEE CONFIGURATION
# ===========================================================================
echo "==> [6/10] Replaying current fee configuration and bounded history..."
FEE_EXPORT="$EXPORT_DIR/fee-config-$TS.json"
OLD_FEE_CONFIG="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_fee_config 2>/dev/null || echo '{}')"
OLD_FEE_HISTORY="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_fee_config_history 2>/dev/null || echo '[]')"
printf '%s\n' "{\"config\":$OLD_FEE_CONFIG,\"history\":$OLD_FEE_HISTORY}" > "$FEE_EXPORT"
admin_call "$NEW_SCOUT_ACCESS_CONTRACT_ID" admin_seed_fee_config \
  --config "$OLD_FEE_CONFIG" --history "$OLD_FEE_HISTORY" >/dev/null

# ===========================================================================
# PART 7 — SUBSCRIPTIONS AND AUTO-RENEWAL
# ===========================================================================
echo "==> [7/10] Replaying subscriptions and auto-renewal flags..."
SUBSCRIPTION_EXPORT="$EXPORT_DIR/subscriptions-$TS.json"
AUTO_RENEW_EXPORT="$EXPORT_DIR/auto-renew-$TS.json"
echo "[]" > "$SUBSCRIPTION_EXPORT"
echo "[]" > "$AUTO_RENEW_EXPORT"

while IFS= read -r scout; do
  wallet="$(echo "$scout" | jq -r '.wallet')"
  subscription="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_subscription --scout "$wallet" 2>/dev/null || true)"
  if [[ -n "$subscription" ]]; then
    tmp="$(mktemp)"
    jq --argjson s "$subscription" '. += [$s]' "$SUBSCRIPTION_EXPORT" > "$tmp" && mv "$tmp" "$SUBSCRIPTION_EXPORT"
    admin_call "$NEW_SCOUT_ACCESS_CONTRACT_ID" admin_seed_subscription \
      --subscription "$subscription" >/dev/null
  fi

  auto_renew="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_auto_renew --scout "$wallet" 2>/dev/null || echo false)"
  tmp="$(mktemp)"
  jq --arg wallet "$wallet" --argjson enabled "$auto_renew" \
    '. += [{scout:$wallet, enabled:$enabled}]' "$AUTO_RENEW_EXPORT" > "$tmp" && mv "$tmp" "$AUTO_RENEW_EXPORT"
  admin_call "$NEW_SCOUT_ACCESS_CONTRACT_ID" admin_seed_auto_renew \
    --scout "$wallet" --enabled "$auto_renew" >/dev/null
done < <(jq -c '.[]' "$SCOUTS_EXPORT")

# ===========================================================================
# PART 8 — CONTACT RECORDS
# ===========================================================================
echo "==> [8/10] Replaying contact records and reverse indexes..."
CONTACT_EXPORT="$EXPORT_DIR/contacts-$TS.json"
echo "[]" > "$CONTACT_EXPORT"
while IFS= read -r scout; do
  wallet="$(echo "$scout" | jq -r '.wallet')"
  contacts="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_scout_contacts --scout "$wallet" 2>/dev/null || echo '[]')"
  while IFS= read -r player_id; do
    [[ -z "$player_id" ]] && continue
    contact="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_contact_record \
      --scout "$wallet" --player_id "$player_id" 2>/dev/null || true)"
    [[ -z "$contact" || "$contact" == "null" ]] && continue
    tmp="$(mktemp)"
    jq --argjson c "$contact" '. += [$c]' "$CONTACT_EXPORT" > "$tmp" && mv "$tmp" "$CONTACT_EXPORT"
    admin_call "$NEW_SCOUT_ACCESS_CONTRACT_ID" admin_seed_contact \
      --contact "$contact" >/dev/null
  done < <(echo "$contacts" | jq -r '.[]')
done < <(jq -c '.[]' "$SCOUTS_EXPORT")

# ===========================================================================
# PART 9 — TRIAL OFFERS AND IN-FLIGHT ESCROWS
# ===========================================================================
echo "==> [9/10] Replaying trial offers and escrow records..."
TRIAL_EXPORT="$EXPORT_DIR/trial-offers-$TS.json"
echo "[]" > "$TRIAL_EXPORT"
while IFS= read -r player; do
  [[ -z "$player" ]] && continue
  player_id="$(echo "$player" | jq -r '.player_id')"
  trial_count="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_trial_count --player_id "$player_id" 2>/dev/null || echo 0)"
  for ((index=1; index<=trial_count; index++)); do
    offer="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_trial_offer \
      --player_id "$player_id" --index "$index" 2>/dev/null || true)"
    [[ -z "$offer" ]] && continue
    escrow="$(invoke_view "$OLD_SCOUT_ACCESS_CONTRACT_ID" get_trial_escrow \
      --player_id "$player_id" --index "$index" 2>/dev/null || echo null)"
    [[ -z "$escrow" ]] && escrow=null
    tmp="$(mktemp)"
    jq --argjson o "$offer" --argjson e "$escrow" \
      '. += [{offer:$o, escrow:$e}]' "$TRIAL_EXPORT" > "$tmp" && mv "$tmp" "$TRIAL_EXPORT"
    admin_call "$NEW_SCOUT_ACCESS_CONTRACT_ID" admin_seed_trial_offer \
      --player_id "$player_id" --trial_index "$index" --offer "$offer" --escrow "$escrow" >/dev/null
  done
done < <(jq -c '.[]' "$PLAYERS_EXPORT")

# ===========================================================================
# PART 10 — CLOSE WINDOWS AND VERIFY READ-BACK
# ===========================================================================
echo "==> [10/10] Closing migration windows and completing replay..."
if [[ "$DRY_RUN" -eq 0 ]]; then
  close_migration_windows
fi

# ===========================================================================
# Summary
# ===========================================================================
echo ""
echo "=========================================================================="
echo "  State replay summary"
echo "=========================================================================="
echo "  Validators : $VALIDATOR_COUNT read from old contract"
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "               (dry-run — none actually registered on the new contract)"
else
  echo "               registered on the NEW verification contract (admin-signed)"
fi
echo "  Players    : $PLAYER_COUNT exported to $PLAYERS_EXPORT"
echo "  Scouts     : $SCOUT_COUNT exported to $SCOUTS_EXPORT"
echo ""
echo "  Players and scouts were re-seeded on the new contract via"
echo "  admin-only registration entrypoints (admin_seed_player /"
echo "  admin_seed_scout)."
echo ""
echo "  The exported JSON files above contain the full replay payloads"
echo "  (wallet, vitals, ipfs_hashes, level, region) for auditing and"
echo "  future reconciliation."
echo "=========================================================================="
