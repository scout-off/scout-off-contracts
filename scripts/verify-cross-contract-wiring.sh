#!/usr/bin/env bash
# ScoutChain — verify cross-contract wiring after deployment or upgrade.
#
# Sources .env.contracts, calls health() on every contract to confirm liveness,
# then calls get_wiring_state() on every contract that exposes it (all four,
# as of issue #1041) to cross-check every peer-address pointer against the
# other pointers/contracts that should agree with it.
#
# For a contract that has not yet been upgraded to expose get_wiring_state()
# (an older deployment predating issue #1041), the script falls back to a
# health-only check for that contract's links and reports them as unverified
# rather than failing outright — see docs/WIRING_REGISTRY_DESIGN.md's
# migration note.
#
# Usage:
#   ./scripts/verify-cross-contract-wiring.sh [testnet|mainnet|local] [--repair]
#
#   --repair   Print only the actionable corrective `stellar contract invoke`
#              commands for every link that is not fully wired, instead of
#              the full report. Prints nothing (and exits 0) if everything is
#              already consistent. This never invokes anything itself — see
#              "Why detect-and-guide, not auto-repair" below.
#
# Prerequisites:
#   • .env.contracts must exist (written by deploy.sh)
#   • stellar-cli must be on PATH
#   • python3 must be on PATH
#
# Exit codes:
#   0  — all checks passed (fully wired, or only WARN-level gaps)
#   1  — one or more checks failed (misconfigured or partially-wired links)
#
# --- Detection model (see docs/WIRING_REGISTRY_DESIGN.md for the full writeup) ---
#
# The four contracts hold eight peer-address pointers between them (not five —
# the original design doc undercounted verification.RegistrationContract and
# scout_access.RegistrationContract, both added by issue #1014 after the doc
# was written). Every pointer targets exactly one of the four contracts, and
# every contract's `get_wiring_state()` now returns, for each pointer it
# holds, both the stored `address` and a `epoch` (bump count incremented on
# every successful set/update call — see scoutchain_shared_types::WiringLink).
#
# Pointers are grouped by the contract they target (not by which contract
# holds them) because that is the natural "should agree" unit: e.g. all
# three pointers naming a ProgressContract (held by verification,
# registration, and scout_access respectively) should each equal the actual
# deployed $PROGRESS_CONTRACT_ID. A classic partial re-wiring — an operator
# redeploys progress and updates two of those three pointers before a script
# crashes or an auth failure interrupts the third — shows up as exactly this
# group having some WIRED members and some STALE/UNCONFIGURED members, which
# this script reports as PARTIAL: a distinct, actionable state from either
# "fully wired" or "never configured at all".
#
# Per-link classification:
#   WIRED         — address is set and equals the target's actual deployed ID
#   MISCONFIGURED — address is set but does NOT equal the target's actual ID
#   UNCONFIGURED  — address was never set (epoch == 0)
#
# Per-target-group rollup (all links naming the same target contract):
#   FULLY_WIRED      — every link in the group is WIRED
#   NEVER_CONFIGURED — every link in the group is UNCONFIGURED
#   PARTIAL          — any other mix — the actionable "one side updated, the
#                      other stale" state this script exists to catch
#
# `epoch` is not needed to tell UNCONFIGURED apart from the other two states
# (address's None/Some already does that) — its value is operator
# diagnostics: comparing the epoch this script reports against a value from a
# previous run tells an operator whether a re-wiring call they just made
# actually landed (epoch increased) or not (epoch unchanged — an auth/network
# problem, not a wrong-address problem). The script does not persist state
# between runs itself; it reports the current epoch so an operator can do
# that comparison by hand or by diffing two `--json`-style runs (see
# docs/WIRING_REGISTRY_DESIGN.md for why an epoch-diff baseline file was
# deliberately not built into this script).
#
# --- Why detect-and-guide, not auto-repair ---
#
# Soroban has no atomic multi-contract transaction primitive available to
# these admin scripts (each `stellar contract invoke` is a separate,
# independently-failable call). Auto-correcting a detected PARTIAL state by
# silently invoking the missing `set_*_contract` calls would mean this script
# — not a human operator — decides to mutate admin-controlled contract state,
# and if ITS OWN corrective calls partially fail, there is no longer any
# distinction between "the original interrupted re-wiring" and "this script's
# own interrupted repair attempt". Detection stays fast and unambiguous;
# repair stays a human action, guided by the exact command this script prints.

set -euo pipefail

NETWORK="testnet"
REPAIR=0
for arg in "$@"; do
  case "$arg" in
    --repair) REPAIR=1 ;;
    testnet|mainnet|local) NETWORK="$arg" ;;
    *)
      echo "Usage: $0 [testnet|mainnet|local] [--repair]" >&2
      exit 1
      ;;
  esac
done

# `stellar contract invoke` requires a --source-account even for the
# read-only `get_wiring_state()` / `health()` probes below. Any funded
# account works for a simulation-only read; callers set
# STELLAR_SOURCE_ACCOUNT (identity name or secret key), falling back to
# DEPLOYER_SECRET.
SOURCE_ACCOUNT="${STELLAR_SOURCE_ACCOUNT:-${DEPLOYER_SECRET:-}}"
if [[ -z "$SOURCE_ACCOUNT" ]]; then
  echo "ERROR: set STELLAR_SOURCE_ACCOUNT (or DEPLOYER_SECRET) to a Stellar identity or secret key." >&2
  exit 1
fi

# shellcheck source=/dev/null
[[ -f .env.contracts ]] && source .env.contracts
for var in REGISTRATION_CONTRACT_ID VERIFICATION_CONTRACT_ID PROGRESS_CONTRACT_ID SCOUT_ACCESS_CONTRACT_ID; do
  if [[ -z "${!var:-}" ]]; then
    echo "ERROR: $var is not set — did you run deploy.sh?" >&2
    exit 1
  fi
done

PASS=0
FAIL=0
WARN=0
REPAIR_CMDS=()

pass() { [[ $REPAIR -eq 0 ]] && echo "  ✅ $*"; PASS=$((PASS + 1)); }
fail() { [[ $REPAIR -eq 0 ]] && echo "  ❌ $*"; FAIL=$((FAIL + 1)); }
warn() { [[ $REPAIR -eq 0 ]] && echo "  ⚠️  $*"; WARN=$((WARN + 1)); }

invoke() {
    stellar contract invoke \
        --id "$1" \
        --source "$SOURCE_ACCOUNT" \
        --network "$NETWORK" \
        -- "$2" 2>&1
}

if [[ $REPAIR -eq 0 ]]; then
  echo "============================================"
  echo "  Cross-Contract Wiring Verification"
  echo "  Network: $NETWORK"
  echo "============================================"

  echo ""
  echo "--- Liveness checks ---"
  for label_id in \
      "Registration:$REGISTRATION_CONTRACT_ID" \
      "Verification:$VERIFICATION_CONTRACT_ID" \
      "Progress:$PROGRESS_CONTRACT_ID" \
      "ScoutAccess:$SCOUT_ACCESS_CONTRACT_ID"
  do
      label="${label_id%%:*}"
      contract_id="${label_id##*:}"
      if resp=$(invoke "$contract_id" health 2>&1); then
          initialized=$(echo "$resp" | python3 -c "import sys,json; d=json.load(sys.stdin); print('yes' if d.get('initialized') else 'no')" 2>/dev/null || echo "unknown")
          paused=$(echo "$resp"      | python3 -c "import sys,json; d=json.load(sys.stdin); print('yes' if d.get('paused') else 'no')" 2>/dev/null || echo "unknown")
          if [[ "$paused" == "yes" ]]; then
              warn "$label: alive — initialized=$initialized PAUSED=yes"
          else
              pass "$label: alive — initialized=$initialized paused=no"
          fi
      else
          fail "$label ($contract_id): health() failed — $resp"
      fi
  done
fi

# ---------------------------------------------------------------------------
# get_wiring_state() on all four contracts
# ---------------------------------------------------------------------------
[[ $REPAIR -eq 0 ]] && { echo ""; echo "--- Wiring state ---"; }

fetch_wiring_state() {
  invoke "$1" get_wiring_state 2>&1
}

REG_STATE=$(fetch_wiring_state "$REGISTRATION_CONTRACT_ID") && REG_OK=1 || REG_OK=0
VER_STATE=$(fetch_wiring_state "$VERIFICATION_CONTRACT_ID") && VER_OK=1 || VER_OK=0
PROG_STATE=$(fetch_wiring_state "$PROGRESS_CONTRACT_ID") && PROG_OK=1 || PROG_OK=0
SA_STATE=$(fetch_wiring_state "$SCOUT_ACCESS_CONTRACT_ID") && SA_OK=1 || SA_OK=0

if [[ $REG_OK -eq 0 && $REPAIR -eq 0 ]]; then
  warn "registration: get_wiring_state() not available — contract may need upgrading (see docs/WIRING_REGISTRY_DESIGN.md)."
fi
if [[ $VER_OK -eq 0 && $REPAIR -eq 0 ]]; then
  warn "verification: get_wiring_state() not available — contract may need upgrading."
fi
if [[ $PROG_OK -eq 0 && $REPAIR -eq 0 ]]; then
  warn "progress: get_wiring_state() not available — contract may need upgrading."
fi
if [[ $SA_OK -eq 0 && $REPAIR -eq 0 ]]; then
  warn "scout_access: get_wiring_state() not available — contract may need upgrading."
fi

# ---------------------------------------------------------------------------
# Single Python pass: parse all four states, group by target contract,
# classify each link and each group, and emit two machine-readable streams
# on stdout (one line per fact) that the bash loop below turns into
# pass/fail/warn calls and repair commands. This keeps the
# per-link/per-group classification logic in one place instead of
# reimplementing it four times in bash.
# ---------------------------------------------------------------------------
ANALYSIS=$(REG_OK="$REG_OK" VER_OK="$VER_OK" PROG_OK="$PROG_OK" SA_OK="$SA_OK" \
  REG_STATE="$REG_STATE" VER_STATE="$VER_STATE" PROG_STATE="$PROG_STATE" SA_STATE="$SA_STATE" \
  REGISTRATION_CONTRACT_ID="$REGISTRATION_CONTRACT_ID" VERIFICATION_CONTRACT_ID="$VERIFICATION_CONTRACT_ID" \
  PROGRESS_CONTRACT_ID="$PROGRESS_CONTRACT_ID" SCOUT_ACCESS_CONTRACT_ID="$SCOUT_ACCESS_CONTRACT_ID" \
  python3 - <<'PYEOF'
# Reads everything via environment variables (never interpolated into this
# script text) so raw stellar-cli output — which may contain quotes,
# backticks, or $-sequences on a failed invoke — can never be re-parsed by
# the shell or break out of this heredoc.
import json, os

reg_ok, ver_ok, prog_ok, sa_ok = (
    os.environ["REG_OK"] == "1",
    os.environ["VER_OK"] == "1",
    os.environ["PROG_OK"] == "1",
    os.environ["SA_OK"] == "1",
)
reg_id = os.environ["REGISTRATION_CONTRACT_ID"]
ver_id = os.environ["VERIFICATION_CONTRACT_ID"]
prog_id = os.environ["PROGRESS_CONTRACT_ID"]
sa_id = os.environ["SCOUT_ACCESS_CONTRACT_ID"]

def load(ok, env_key):
    if not ok:
        return None
    try:
        return json.loads(os.environ.get(env_key, ""))
    except Exception:
        return None

reg = load(reg_ok, "REG_STATE")
ver = load(ver_ok, "VER_STATE")
prog = load(prog_ok, "PROG_STATE")
sa = load(sa_ok, "SA_STATE")

def flat_link(state, field):
    # progress's get_wiring_state() keeps flat <field>_contract /
    # <field>_epoch fields for backward compatibility (see types.rs).
    if state is None:
        return (None, None)
    return (state.get(f"{field}_contract"), state.get(f"{field}_epoch"))

def nested_link(state, field):
    # verification / registration / scout_access use the shared
    # scoutchain_shared_types::WiringLink { address, epoch } shape.
    if state is None:
        return (None, None)
    link = state.get(f"{field}_contract") or {}
    return (link.get("address"), link.get("epoch"))

# owner, field-on-owner, accessor -> (address, epoch) or (None, None) if the
# owning contract's get_wiring_state() call failed outright (unknown, not
# UNCONFIGURED — reported separately as a WARN above).
LINKS = [
    # (owner_label, owner_available, address, epoch, target_label, target_id)
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
        print(f"LINK\t{owner}\t{target}\tUNAVAILABLE\t-\t-\t{target_id}")
        continue
    status = classify(address, epoch, target_id)
    print(f"LINK\t{owner}\t{target}\t{status}\t{address}\t{epoch}\t{target_id}")
    groups.setdefault(target, []).append(status)

for target, statuses in groups.items():
    non_unavailable = [s for s in statuses if s != "UNAVAILABLE"]
    if not non_unavailable:
        group_status = "UNKNOWN"
    elif all(s == "WIRED" for s in non_unavailable):
        group_status = "FULLY_WIRED"
    elif all(s == "UNCONFIGURED" for s in non_unavailable):
        group_status = "NEVER_CONFIGURED"
    else:
        group_status = "PARTIAL"
    print(f"GROUP\t{target}\t{group_status}")
PYEOF
)

declare -A GROUP_STATUS

while IFS=$'\t' read -r kind a b c d e f; do
  if [[ "$kind" == "GROUP" ]]; then
    GROUP_STATUS["$a"]="$b"
    continue
  fi
  # kind == LINK: a=owner b=target c=status d=address e=epoch f=target_id
  owner="$a"; target="$b"; status="$c"; address="$d"; epoch="$e"; target_id="$f"
  field_name="${target}"
  cmd="stellar contract invoke --id \$${owner^^}_CONTRACT_ID --network $NETWORK --source \$DEPLOYER -- set_${field_name}_contract --addr \$${target^^}_CONTRACT_ID"
  # scout_access/verification's progress link setter is named
  # set_progress_contract regardless of field naming above, which already
  # matches; registration's link is also set_progress_contract. No special
  # casing is needed here because every setter is literally named
  # set_<target>_contract on its owning contract.
  case "$status" in
    WIRED)
      pass "$owner.${field_name}_contract → $target: $address (epoch $epoch) ✓"
      ;;
    UNCONFIGURED)
      fail "$owner.${field_name}_contract → $target: NOT SET (run: $cmd)"
      REPAIR_CMDS+=("$cmd")
      ;;
    MISCONFIGURED)
      fail "$owner.${field_name}_contract → $target: $address ≠ expected $target_id (epoch $epoch — run: $cmd)"
      REPAIR_CMDS+=("$cmd")
      ;;
    UNAVAILABLE)
      # Already warned about above at the per-contract get_wiring_state() level.
      :
      ;;
  esac
done <<< "$ANALYSIS"

if [[ $REPAIR -eq 0 ]]; then
  echo ""
  echo "--- Per-target-contract consistency (partial-rewiring detection) ---"
  for target in progress registration verification scout_access; do
    gs="${GROUP_STATUS[$target]:-UNKNOWN}"
    case "$gs" in
      FULLY_WIRED)
        pass "$target: all pointers naming it agree — fully wired"
        ;;
      NEVER_CONFIGURED)
        warn "$target: no dependent has ever wired a pointer to it yet"
        ;;
      PARTIAL)
        fail "$target: PARTIAL — some dependents point at it correctly, others do not (see links above). This is the interrupted/partial re-wiring case — see docs/WIRING_REGISTRY_DESIGN.md."
        ;;
      UNKNOWN)
        warn "$target: could not be assessed (no owning contract's get_wiring_state() succeeded for this target)"
        ;;
    esac
  done
fi

if [[ $REPAIR -eq 1 ]]; then
  if [[ ${#REPAIR_CMDS[@]} -eq 0 ]]; then
    exit 0
  fi
  echo "The following corrective calls will bring wiring to a consistent state."
  echo "Each is independent — run them in any order, then re-run this script"
  echo "(without --repair) to confirm. This script does not invoke these for"
  echo "you; see \"Why detect-and-guide, not auto-repair\" at the top of this"
  echo "file for why."
  echo ""
  for cmd in "${REPAIR_CMDS[@]}"; do
    echo "  $cmd"
  done
  exit 1
fi

echo ""
echo "============================================"
echo "  Results: $PASS passed, $FAIL failed, $WARN warnings"
echo "============================================"

if [[ "$FAIL" -gt 0 ]]; then
    echo ""
    echo "  One or more wiring links are broken, misconfigured, or partially re-wired."
    echo "  Run with --repair for the exact corrective commands, or fix the links shown"
    echo "  above before routing live traffic to these contracts."
    exit 1
fi

echo ""
echo "  All verified links are correctly and consistently wired."
if [[ "$WARN" -gt 0 ]]; then
    echo "  ($WARN warning(s) — see above for not-yet-upgraded contracts or never-configured links)"
fi
