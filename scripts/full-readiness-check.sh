#!/usr/bin/env bash
# ScoutChain — full post-deploy readiness check
#
# Combines health-check.sh (init/pause status for all four contracts) and
# verify-cross-contract-wiring.sh (all eight cross-contract wiring links,
# grouped by target contract to detect a partial re-wiring — see
# docs/WIRING_REGISTRY_DESIGN.md) into a single pass/fail command.  Run this
# after every deployment or upgrade to confirm all contracts are healthy and
# correctly wired before routing traffic.
#
# Usage:
#   ./scripts/full-readiness-check.sh [testnet|mainnet|local]
#
# Prerequisites:
#   • .env.contracts must exist (written by deploy.sh)
#   • stellar-cli must be on PATH
#
# Exit codes:
#   0  — all health and wiring checks passed
#   1  — one or more checks failed (see summary table for details)
#
# See also:
#   scripts/health-check.sh              — health-only variant
#   scripts/verify-cross-contract-wiring.sh — wiring-only variant
#   docs/DEPLOYMENT.md                   — deployment guide and post-deploy checklist

set -euo pipefail

NETWORK="${1:-testnet}"

# `stellar contract invoke` requires a --source-account even for the
# read-only health()/get_wiring_state() probes below. Any funded account
# works for a simulation-only read; callers set STELLAR_SOURCE_ACCOUNT
# (identity name or secret key), falling back to DEPLOYER_SECRET.
SOURCE_ACCOUNT="${STELLAR_SOURCE_ACCOUNT:-${DEPLOYER_SECRET:-}}"
if [[ -z "$SOURCE_ACCOUNT" ]]; then
    echo "ERROR: set STELLAR_SOURCE_ACCOUNT (or DEPLOYER_SECRET) to a Stellar identity or secret key." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Load contract IDs from .env.contracts
# ---------------------------------------------------------------------------
if [[ ! -f .env.contracts ]]; then
    echo "ERROR: .env.contracts not found — did you run deploy.sh?" >&2
    exit 1
fi
# shellcheck source=/dev/null
source .env.contracts

for var in REGISTRATION_CONTRACT_ID VERIFICATION_CONTRACT_ID PROGRESS_CONTRACT_ID SCOUT_ACCESS_CONTRACT_ID; do
    if [[ -z "${!var:-}" ]]; then
        echo "ERROR: $var is not set in .env.contracts — did you run deploy.sh?" >&2
        exit 1
    fi
done

# ---------------------------------------------------------------------------
# Result tracking
# ---------------------------------------------------------------------------
PASS=0
FAIL=0
WARN=0

# Arrays to accumulate per-check results for the combined summary table.
# Each entry is: "STATUS|CHECK_NAME|DETAIL"
declare -a RESULTS=()

record_pass() {
    local check_name="$1"
    local detail="${2:-}"
    PASS=$((PASS + 1))
    RESULTS+=("PASS|${check_name}|${detail}")
}

record_fail() {
    local check_name="$1"
    local detail="${2:-}"
    FAIL=$((FAIL + 1))
    RESULTS+=("FAIL|${check_name}|${detail}")
}

record_warn() {
    local check_name="$1"
    local detail="${2:-}"
    WARN=$((WARN + 1))
    RESULTS+=("WARN|${check_name}|${detail}")
}

invoke() {
    stellar contract invoke \
        --id "$1" \
        --source "$SOURCE_ACCOUNT" \
        --network "$NETWORK" \
        -- "$2" 2>&1
}

echo "============================================"
echo "  ScoutChain Full Readiness Check"
echo "  Network: $NETWORK"
echo "============================================"

# ===========================================================================
# SECTION 1 — Health checks (init/pause status for all four contracts)
# Mirrors the logic in scripts/health-check.sh without calling it as a
# subprocess, so output can be incorporated into the combined summary table.
# ===========================================================================
echo ""
echo "--- Section 1: Contract health (initialized & not paused) ---"

declare -A CONTRACT_IDS=(
    [registration]="$REGISTRATION_CONTRACT_ID"
    [verification]="$VERIFICATION_CONTRACT_ID"
    [progress]="$PROGRESS_CONTRACT_ID"
    [scout_access]="$SCOUT_ACCESS_CONTRACT_ID"
)

HEALTH_CONTRACT_ORDER=(registration verification progress scout_access)

for name in "${HEALTH_CONTRACT_ORDER[@]}"; do
    id="${CONTRACT_IDS[$name]}"
    echo "==> health() on ${name} (${id})..."
    check_label="health:${name}"

    if response=$(invoke "$id" health 2>&1); then
        echo "    Response: $response"

        if echo "$response" | grep -q '"initialized":false'; then
            echo "    ❌ FAIL: ${name} returned initialized: false"
            record_fail "$check_label" "initialized: false — run initialize.sh"
        elif echo "$response" | grep -q '"paused":true'; then
            echo "    ❌ FAIL: ${name} returned paused: true"
            record_fail "$check_label" "paused: true — call unpause_contract to resume"
        elif echo "$response" | grep -q '"initialized":true'; then
            echo "    ✅ OK: ${name} is healthy"
            record_pass "$check_label" "initialized: true, paused: false"
        else
            echo "    ❌ FAIL: ${name} returned unexpected health response"
            record_fail "$check_label" "unexpected health response: ${response}"
        fi
    else
        echo "    ❌ FAIL: ${name} health() call failed — ${response}"
        record_fail "$check_label" "health() invocation failed: ${response}"
    fi
done

# ===========================================================================
# SECTION 2 — Cross-contract wiring verification
#
# Classification logic (WIRED / MISCONFIGURED / UNCONFIGURED per link,
# FULLY_WIRED / NEVER_CONFIGURED / PARTIAL per target-contract group) mirrors
# scripts/verify-cross-contract-wiring.sh exactly — see that script's header
# comment for the full design rationale (why groups are keyed by target
# contract, what `epoch` is for, why detection doesn't auto-repair). Kept as
# an inline duplicate here (not a subprocess call) so per-link results feed
# this script's own combined summary table below; keep both in sync if the
# classification logic changes.
# ===========================================================================
echo ""
echo "--- Section 2: Cross-contract wiring (all 8 links) ---"

REG_WIRING_OK=0; VER_WIRING_OK=0; PROG_WIRING_OK=0; SA_WIRING_OK=0
REG_STATE=$(invoke "$REGISTRATION_CONTRACT_ID" get_wiring_state 2>&1) && REG_WIRING_OK=1
VER_STATE=$(invoke "$VERIFICATION_CONTRACT_ID" get_wiring_state 2>&1) && VER_WIRING_OK=1
PROG_STATE=$(invoke "$PROGRESS_CONTRACT_ID" get_wiring_state 2>&1) && PROG_WIRING_OK=1
SA_STATE=$(invoke "$SCOUT_ACCESS_CONTRACT_ID" get_wiring_state 2>&1) && SA_WIRING_OK=1

for entry in \
  "registration:$REG_WIRING_OK" "verification:$VER_WIRING_OK" \
  "progress:$PROG_WIRING_OK" "scout_access:$SA_WIRING_OK"; do
  name="${entry%%:*}"; ok="${entry##*:}"
  if [[ "$ok" -eq 0 ]]; then
    echo "  ⚠️  ${name}: get_wiring_state() not available — contract may need upgrading."
    record_warn "wiring:${name}:getter" "get_wiring_state() not available on ${name} contract"
  fi
done

WIRING_ANALYSIS=$(REG_OK="$REG_WIRING_OK" VER_OK="$VER_WIRING_OK" PROG_OK="$PROG_WIRING_OK" SA_OK="$SA_WIRING_OK" \
  REG_STATE="$REG_STATE" VER_STATE="$VER_STATE" PROG_STATE="$PROG_STATE" SA_STATE="$SA_STATE" \
  REGISTRATION_CONTRACT_ID="$REGISTRATION_CONTRACT_ID" VERIFICATION_CONTRACT_ID="$VERIFICATION_CONTRACT_ID" \
  PROGRESS_CONTRACT_ID="$PROGRESS_CONTRACT_ID" SCOUT_ACCESS_CONTRACT_ID="$SCOUT_ACCESS_CONTRACT_ID" \
  python3 - <<'PYEOF'
import json, os

reg_ok, ver_ok, prog_ok, sa_ok = (
    os.environ["REG_OK"] == "1", os.environ["VER_OK"] == "1",
    os.environ["PROG_OK"] == "1", os.environ["SA_OK"] == "1",
)
reg_id, ver_id = os.environ["REGISTRATION_CONTRACT_ID"], os.environ["VERIFICATION_CONTRACT_ID"]
prog_id, sa_id = os.environ["PROGRESS_CONTRACT_ID"], os.environ["SCOUT_ACCESS_CONTRACT_ID"]

def load(ok, key):
    if not ok:
        return None
    try:
        return json.loads(os.environ.get(key, ""))
    except Exception:
        return None

reg, ver, prog, sa = load(reg_ok, "REG_STATE"), load(ver_ok, "VER_STATE"), load(prog_ok, "PROG_STATE"), load(sa_ok, "SA_STATE")

def flat_link(state, field):
    if state is None:
        return (None, None)
    return (state.get(f"{field}_contract"), state.get(f"{field}_epoch"))

def nested_link(state, field):
    if state is None:
        return (None, None)
    link = state.get(f"{field}_contract") or {}
    return (link.get("address"), link.get("epoch"))

LINKS = [
    ("verification", ver_ok, *nested_link(ver, "progress"), "progress", prog_id),
    ("registration", reg_ok, *nested_link(reg, "progress"), "progress", prog_id),
    ("scout_access", sa_ok, *nested_link(sa, "progress"), "progress", prog_id),
    ("verification", ver_ok, *nested_link(ver, "registration"), "registration", reg_id),
    ("progress", prog_ok, *flat_link(prog, "registration"), "registration", reg_id),
    ("scout_access", sa_ok, *nested_link(sa, "registration"), "registration", reg_id),
    ("progress", prog_ok, *flat_link(prog, "verification"), "verification", ver_id),
    ("progress", prog_ok, *flat_link(prog, "scout_access"), "scout_access", sa_id),
]

def classify(address, epoch, expected_id):
    if address is None or not epoch:
        return "UNCONFIGURED"
    if address == expected_id:
        return "WIRED"
    return "MISCONFIGURED"

groups = {}
for owner, owner_available, address, epoch, target, target_id in LINKS:
    if not owner_available:
        continue
    status = classify(address, epoch, target_id)
    print(f"LINK\t{owner}\t{target}\t{status}\t{address}\t{epoch}\t{target_id}")
    groups.setdefault(target, []).append(status)

for target, statuses in groups.items():
    if all(s == "WIRED" for s in statuses):
        group_status = "FULLY_WIRED"
    elif all(s == "UNCONFIGURED" for s in statuses):
        group_status = "NEVER_CONFIGURED"
    else:
        group_status = "PARTIAL"
    print(f"GROUP\t{target}\t{group_status}")
PYEOF
)

declare -A WIRING_GROUP_STATUS
while IFS=$'\t' read -r kind a b c d e f; do
  if [[ "$kind" == "GROUP" ]]; then
    WIRING_GROUP_STATUS["$a"]="$b"
    continue
  fi
  owner="$a"; target="$b"; status="$c"; address="$d"; epoch="$e"; target_id="$f"
  cmd="stellar contract invoke --id \$${owner^^}_CONTRACT_ID --network $NETWORK --source \$DEPLOYER -- set_${target}_contract --addr \$${target^^}_CONTRACT_ID"
  case "$status" in
    WIRED)
      echo "  ✅ OK: ${owner} → ${target}_contract: ${address} (epoch ${epoch})"
      record_pass "wiring:${owner}→${target}" "${address} matches, epoch ${epoch}"
      ;;
    UNCONFIGURED)
      echo "  ❌ FAIL: ${owner} → ${target}_contract: NOT SET"
      record_fail "wiring:${owner}→${target}" "NOT SET — run: ${cmd}"
      ;;
    MISCONFIGURED)
      echo "  ❌ FAIL: ${owner} → ${target}_contract: ${address} ≠ expected ${target_id}"
      record_fail "wiring:${owner}→${target}" "${address} ≠ expected ${target_id} (epoch ${epoch} — run: ${cmd})"
      ;;
  esac
done <<< "$WIRING_ANALYSIS"

echo ""
echo "  Per-target-contract consistency (partial-rewiring detection):"
for target in progress registration verification scout_access; do
  gs="${WIRING_GROUP_STATUS[$target]:-UNKNOWN}"
  case "$gs" in
    FULLY_WIRED)
      echo "  ✅ ${target}: all pointers naming it agree"
      ;;
    NEVER_CONFIGURED)
      echo "  ⚠️  ${target}: no dependent has ever wired a pointer to it yet"
      record_warn "wiring-group:${target}" "never configured"
      ;;
    PARTIAL)
      echo "  ❌ FAIL: ${target}: PARTIAL — some dependents point at it correctly, others do not"
      record_fail "wiring-group:${target}" "partial re-wiring detected — run scripts/verify-cross-contract-wiring.sh --repair"
      ;;
    UNKNOWN)
      : # No owning contract's get_wiring_state() succeeded for this target; already warned above.
      ;;
  esac
done

# ===========================================================================
# COMBINED SUMMARY TABLE
# ===========================================================================
echo ""
echo "============================================"
echo "  Full Readiness Check — Combined Summary"
echo "  Network: $NETWORK"
echo "============================================"
printf "  %-42s  %-6s  %s\n" "CHECK" "STATUS" "DETAIL"
printf "  %-42s  %-6s  %s\n" "$(printf '%0.s-' {1..42})" "------" "------"

for entry in "${RESULTS[@]}"; do
    IFS='|' read -r status check_name detail <<< "$entry"
    case "$status" in
        PASS) icon="✅ PASS" ;;
        FAIL) icon="❌ FAIL" ;;
        WARN) icon="⚠️  WARN" ;;
        *)    icon="$status" ;;
    esac
    printf "  %-42s  %-6s  %s\n" "$check_name" "$icon" "$detail"
done

echo ""
echo "  Totals: ${PASS} passed, ${FAIL} failed, ${WARN} warnings"
echo "============================================"

# ---------------------------------------------------------------------------
# Exit with failure if any check failed, clearly naming the cause.
# Warnings (pending Step 2 upgrades) do not cause a non-zero exit.
# ---------------------------------------------------------------------------
if [[ "$FAIL" -gt 0 ]]; then
    echo ""
    echo "  RESULT: ❌ READINESS CHECK FAILED"
    echo ""
    echo "  The following checks failed:"
    for entry in "${RESULTS[@]}"; do
        IFS='|' read -r status check_name detail <<< "$entry"
        if [[ "$status" == "FAIL" ]]; then
            echo "    ❌ ${check_name}: ${detail}"
        fi
    done
    echo ""
    echo "  Fix the issues above, then re-run: ./scripts/full-readiness-check.sh ${NETWORK}"
    exit 1
fi

echo ""
echo "  RESULT: ✅ ALL CHECKS PASSED"
if [[ "$WARN" -gt 0 ]]; then
    echo "  (${WARN} warning(s) — pending Step 2 wiring upgrades; see docs/WIRING_REGISTRY_DESIGN.md)"
fi
