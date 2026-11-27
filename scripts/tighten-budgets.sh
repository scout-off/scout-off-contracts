#!/usr/bin/env bash
# ScoutChain — suggest tightened values for both CI budget files.
#
# Implements the "measure then tighten" follow-up documented in
# ci/wasm-size-budget.json and ci/cpu-cost-budget.md.  This is the
# one-shot human-review version; a fully-automated ongoing pipeline
# (auto-committing budget bumps in CI) is a separate, larger item.
#
# What this script does
# ─────────────────────
# 1. Builds all contracts (cargo build --workspace) and optimises the WASM
#    outputs with `stellar contract optimize`, then reads the optimised file
#    sizes.
# 2. Runs the existing cost-budget regression tests with --nocapture to
#    capture the "cost_budget: <contract>::<op> = N cpu instructions" lines
#    emitted by each contracts/*/tests/cost_budget.rs file.
# 3. Combines the measured sizes and CPU costs with a configurable headroom
#    percentage and prints a suggested tightened budget for every entry in
#    both ci/wasm-size-budget.json and ci/cpu-cost-budget.md.
# 4. Exits 0.  It never edits the budget files itself — the output is for a
#    human to review and apply (or feed into a follow-up automation step).
#
# Usage
# ─────
# ./scripts/tighten-budgets.sh [--headroom-pct <N>] [--wasm-only] [--cpu-only]
#
# Options:
#   --headroom-pct <N>   Integer percentage of headroom above the measured
#                        value.  Suggested budget = ceil(measured * (1 + N/100)).
#                        Default: 20
#   --wasm-only          Skip the CPU cost budget section.
#   --cpu-only           Skip the WASM size budget section (no build/optimize).
#   --no-color           Suppress ANSI colour codes (implied when stdout is
#                        not a terminal).
#
# Prerequisites
# ─────────────
# - Rust + the wasm32v1-none target   (rustup target add wasm32v1-none)
# - stellar CLI on PATH               (for `stellar contract optimize`)
# - cargo on PATH
#
# The script exits non-zero (code 1) only for hard failures such as missing
# tools or a build error — a contract exceeding its *current* budget does
# not cause a non-zero exit here; that is the job of the CI test step.

set -euo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# Argument parsing
# ──────────────────────────────────────────────────────────────────────────────
HEADROOM_PCT=20
DO_WASM=true
DO_CPU=true
NO_COLOR=false

if [[ ! -t 1 ]]; then
  NO_COLOR=true
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --headroom-pct)
      HEADROOM_PCT="$2"
      shift 2
      ;;
    --wasm-only)
      DO_CPU=false
      shift
      ;;
    --cpu-only)
      DO_WASM=false
      shift
      ;;
    --no-color)
      NO_COLOR=true
      shift
      ;;
    *)
      echo "Unknown option: $1" >&2
      echo "Usage: $0 [--headroom-pct N] [--wasm-only] [--cpu-only] [--no-color]" >&2
      exit 1
      ;;
  esac
done

if ! [[ "$HEADROOM_PCT" =~ ^[0-9]+$ ]]; then
  echo "ERROR: --headroom-pct must be a non-negative integer, got: $HEADROOM_PCT" >&2
  exit 1
fi

# ──────────────────────────────────────────────────────────────────────────────
# Colour helpers (gracefully degrades when NO_COLOR=true)
# ──────────────────────────────────────────────────────────────────────────────
if [[ "$NO_COLOR" == "true" ]]; then
  bold=""  green=""  yellow=""  cyan=""  reset=""
else
  bold="\033[1m"
  green="\033[32m"
  yellow="\033[33m"
  cyan="\033[36m"
  reset="\033[0m"
fi

header() { echo -e "\n${bold}${cyan}==> $*${reset}"; }
info()   { echo -e "    $*"; }
ok()     { echo -e "    ${green}✔${reset}  $*"; }
warn()   { echo -e "    ${yellow}⚠${reset}  $*"; }

# ──────────────────────────────────────────────────────────────────────────────
# Prerequisite checks
# ──────────────────────────────────────────────────────────────────────────────
header "Checking prerequisites"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: '$1' not found on PATH.  $2" >&2
    exit 1
  fi
  ok "$1 found"
}

if [[ "$DO_WASM" == "true" ]]; then
  require_cmd cargo  "Install Rust: https://rustup.rs"
  require_cmd stellar "Install stellar CLI: https://developers.stellar.org/docs/tools/developer-tools/cli/install-stellar-cli"

  # Check wasm32v1-none target is installed
  if ! rustup target list --installed 2>/dev/null | grep -q "wasm32v1-none"; then
    # Older toolchain may still use wasm32-unknown-unknown
    if ! rustup target list --installed 2>/dev/null | grep -q "wasm32-unknown-unknown"; then
      echo "ERROR: Neither wasm32v1-none nor wasm32-unknown-unknown Rust target is installed." >&2
      echo "       Run: rustup target add wasm32v1-none" >&2
      exit 1
    fi
    WASM_TARGET="wasm32-unknown-unknown"
    warn "wasm32v1-none not found; falling back to wasm32-unknown-unknown"
  else
    WASM_TARGET="wasm32v1-none"
    ok "wasm32v1-none target available"
  fi
fi

if [[ "$DO_CPU" == "true" ]]; then
  require_cmd cargo "Install Rust: https://rustup.rs"
fi

# ──────────────────────────────────────────────────────────────────────────────
# Resolve repo root (script may be invoked from any directory)
# ──────────────────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

CONTRACTS=(registration verification progress scout_access)
WASM_DIR="target/${WASM_TARGET}/release"

# ──────────────────────────────────────────────────────────────────────────────
# Step 1: WASM size measurement
# ──────────────────────────────────────────────────────────────────────────────
declare -A MEASURED_WASM_BYTES

if [[ "$DO_WASM" == "true" ]]; then
  header "Building contracts (cargo build --workspace --target $WASM_TARGET --release)"
  cargo build --workspace --target "$WASM_TARGET" --release 2>&1 | tail -5
  ok "Build complete"

  header "Optimising and measuring WASM sizes"
  for name in "${CONTRACTS[@]}"; do
    src_wasm="${WASM_DIR}/scoutchain_${name}.wasm"
    opt_wasm="${WASM_DIR}/scoutchain_${name}.optimized.wasm"

    if [[ ! -f "$src_wasm" ]]; then
      warn "No WASM found for $name at $src_wasm — skipping"
      MEASURED_WASM_BYTES[$name]=0
      continue
    fi

    info "Optimising $name …"
    stellar contract optimize --wasm "$src_wasm" --wasm-out "$opt_wasm" 2>/dev/null

    if [[ -f "$opt_wasm" ]]; then
      bytes=$(wc -c < "$opt_wasm" | tr -d ' ')
    else
      # stellar optimize may write to the same path; fall back to unoptimised
      bytes=$(wc -c < "$src_wasm" | tr -d ' ')
      warn "Optimised WASM not found; using unoptimised size for $name"
    fi

    MEASURED_WASM_BYTES[$name]=$bytes
    ok "$name: ${bytes} bytes"
  done
fi

# ──────────────────────────────────────────────────────────────────────────────
# Step 2: CPU cost measurement via cost_budget tests
# ──────────────────────────────────────────────────────────────────────────────
# The tests emit lines matching:
#   cost_budget: <contract>::<op> = N cpu instructions (budget B)
# We capture those lines and parse them.

declare -A MEASURED_CPU  # key: "contract::op"  value: measured cpu instructions

if [[ "$DO_CPU" == "true" ]]; then
  header "Running cost budget tests (cargo test --workspace --test cost_budget -- --nocapture)"

  # Capture test output; tolerate test failures (we only need the printed numbers)
  cpu_output=$(cargo test --workspace --test cost_budget -- --nocapture 2>&1 || true)

  # Extract lines of the form:
  #   cost_budget: scout_access::subscribe = 12345678 cpu instructions (budget 20000000)
  while IFS= read -r line; do
    if [[ "$line" =~ cost_budget:[[:space:]]([a-z_]+)::([a-z_]+)[[:space:]]=[[:space:]]([0-9]+)[[:space:]]cpu ]]; then
      contract="${BASH_REMATCH[1]}"
      op="${BASH_REMATCH[2]}"
      cost="${BASH_REMATCH[3]}"
      MEASURED_CPU["${contract}::${op}"]="$cost"
      ok "${contract}::${op} = ${cost} cpu instructions"
    fi
  done <<< "$cpu_output"

  if [[ ${#MEASURED_CPU[@]} -eq 0 ]]; then
    warn "No cost_budget output lines were captured."
    warn "Ensure contracts/*/tests/cost_budget.rs tests pass and emit the expected lines."
    warn "CPU budget suggestions will be skipped."
  fi
fi

# ──────────────────────────────────────────────────────────────────────────────
# Helper: ceiling division — ceil(a * (1 + pct/100))
# ──────────────────────────────────────────────────────────────────────────────
# Uses only integer arithmetic (bash).
suggest() {
  local measured=$1
  # suggested = ceil(measured * (100 + HEADROOM_PCT) / 100)
  echo $(( (measured * (100 + HEADROOM_PCT) + 99) / 100 ))
}

# ──────────────────────────────────────────────────────────────────────────────
# Step 3: Print suggested tightened budgets
# ──────────────────────────────────────────────────────────────────────────────
echo ""
echo -e "${bold}════════════════════════════════════════════════════════════════════════${reset}"
echo -e "${bold}  Suggested tightened budgets  (headroom: ${HEADROOM_PCT}%)${reset}"
echo -e "${bold}════════════════════════════════════════════════════════════════════════${reset}"
echo ""
echo "  Review these numbers and apply them manually, or pipe this output"
echo "  into a follow-up script that patches the files directly."
echo ""

# ── WASM size budget (ci/wasm-size-budget.json) ───────────────────────────────
if [[ "$DO_WASM" == "true" ]]; then
  echo -e "${bold}── ci/wasm-size-budget.json ──────────────────────────────────────────────${reset}"
  echo ""
  echo '  Replace the "budgets" block with:'
  echo ""
  echo '  "budgets": {'

  WASM_ANY=false
  for name in "${CONTRACTS[@]}"; do
    bytes="${MEASURED_WASM_BYTES[$name]:-0}"
    if [[ "$bytes" -gt 0 ]]; then
      sug=$(suggest "$bytes")
      printf '    %-16s %s\n' "\"${name}\":" "${sug},"
      WASM_ANY=true
    else
      # No measurement — keep current value
      current_val=$(python3 -c "import json,sys; d=json.load(open('ci/wasm-size-budget.json')); print(d['budgets']['${name}'])" 2>/dev/null || echo "?")
      printf '    %-16s %s   %s\n' "\"${name}\":" "${current_val}," "# ← could not measure; keeping current"
    fi
  done

  echo '  }'
  echo ""

  if [[ "$WASM_ANY" == "true" ]]; then
    echo "  Detail (measured optimised WASM size + ${HEADROOM_PCT}% headroom):"
    printf "  %-20s  %12s  %12s\n" "contract" "measured (B)" "suggested (B)"
    printf "  %-20s  %12s  %12s\n" "────────────────────" "────────────" "─────────────"
    for name in "${CONTRACTS[@]}"; do
      bytes="${MEASURED_WASM_BYTES[$name]:-0}"
      if [[ "$bytes" -gt 0 ]]; then
        sug=$(suggest "$bytes")
        printf "  %-20s  %12s  %12s\n" "$name" "$bytes" "$sug"
      else
        printf "  %-20s  %12s  %12s\n" "$name" "(no data)" "(unchanged)"
      fi
    done
  fi
  echo ""
fi

# ── CPU cost budget (ci/cpu-cost-budget.md) ───────────────────────────────────
if [[ "$DO_CPU" == "true" && ${#MEASURED_CPU[@]} -gt 0 ]]; then
  echo -e "${bold}── ci/cpu-cost-budget.md ─────────────────────────────────────────────────${reset}"
  echo ""
  echo "  Replace the 'Current budgets' table rows with:"
  echo ""

  # Table order as documented in ci/cpu-cost-budget.md
  # Format: "contract_key::op_name" "Display contract" "Display op"
  declare -a TABLE_ROWS=(
    "registration::register_player      registration   register_player"
    "registration::update_profile       registration   update_profile"
    "registration::filter_players       registration   filter_players"
    "verification::register_validator   verification   register_validator"
    "verification::approve_milestone    verification   approve_milestone"
    "verification::get_validator_milestones_page  verification   get_validator_milestones_page"
    "progress::advance_level            progress       advance_level"
    "progress::reset_player_level       progress       reset_player_level"
    "progress::get_progress_history_page progress      get_progress_history_page"
    "scout_access::subscribe            scout_access   subscribe"
    "scout_access::pay_to_contact       scout_access   pay_to_contact"
    "scout_access::batch_contact_players scout_access  batch_contact_players (5 ids)"
    "scout_access::expire_trial_offers  scout_access   expire_trial_offers (limit=20)"
  )

  printf "  | %-15s | %-36s | %s\n" "Contract" "Operation" "Budget (CPU instructions)"
  printf "  |%s|%s|%s\n" "$(printf -- '-%.0s' {1..17})" "$(printf -- '-%.0s' {1..38})" "$(printf -- '-%.0s' {1..26})"

  for row in "${TABLE_ROWS[@]}"; do
    read -r key_raw display_contract display_op_raw <<< "$row"
    # display_op may have spaces — reconstruct from the row after the second token
    display_op=$(echo "$row" | awk '{for(i=3;i<=NF;i++) printf "%s%s", $i, (i<NF?" ":""); print ""}')

    # Normalize key: map display op with spaces to the key format used by test output
    # The test key uses the raw Rust function name (no spaces/parens)
    contract_key=$(echo "$row" | awk '{print $1}')

    measured="${MEASURED_CPU[$contract_key]:-}"

    if [[ -n "$measured" ]]; then
      sug=$(suggest "$measured")
      # Format with commas for readability
      sug_fmt=$(printf "%d" "$sug" | sed ':a;s/\B[0-9]\{3\}\>/,&/;ta')
      printf "  | %-15s | %-36s | %s\n" "$display_contract" "$display_op" "$sug_fmt"
    else
      # No measured value — emit current budget from the md file (best-effort)
      current=$(grep -A1 "$display_op" ci/cpu-cost-budget.md 2>/dev/null | \
                grep -oE '[0-9,]+' | head -1 | tr -d ',' || echo "?")
      printf "  | %-15s | %-36s | %s  # ← no measurement captured\n" \
             "$display_contract" "$display_op" "$current"
    fi
  done

  echo ""
  echo "  Detail (measured CPU instructions + ${HEADROOM_PCT}% headroom):"
  printf "  %-44s  %14s  %14s\n" "contract::op" "measured" "suggested"
  printf "  %-44s  %14s  %14s\n" \
         "$(printf -- '-%.0s' {1..44})" \
         "$(printf -- '-%.0s' {1..14})" \
         "$(printf -- '-%.0s' {1..14})"

  for key in "${!MEASURED_CPU[@]}"; do
    measured="${MEASURED_CPU[$key]}"
    sug=$(suggest "$measured")
    printf "  %-44s  %14s  %14s\n" "$key" "$measured" "$sug"
  done | sort
  echo ""
fi

echo -e "${bold}════════════════════════════════════════════════════════════════════════${reset}"
echo -e "${bold}  Next steps${reset}"
echo -e "${bold}════════════════════════════════════════════════════════════════════════${reset}"
echo ""
echo "  1. Review the suggested values above."
echo "  2. Apply them by editing:"
echo "       ci/wasm-size-budget.json   — update the \"budgets\" object"
echo "       ci/cpu-cost-budget.md      — update the table rows in §Current budgets"
echo "       contracts/*/tests/cost_budget.rs — bump the *_CPU_BUDGET constants"
echo "         to match ci/cpu-cost-budget.md"
echo "  3. Run 'cargo test --workspace' to confirm all budget tests still pass."
echo "  4. Commit the three-file change with a note explaining the tightening."
echo ""
echo "  Headroom used: ${HEADROOM_PCT}%  (override with --headroom-pct N)"
echo ""
