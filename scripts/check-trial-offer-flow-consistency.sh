#!/usr/bin/env bash
# check-trial-offer-flow-consistency.sh
#
# Regression check for issue #794: the trial-offer flow is a two-step
# log-then-confirm flow (`log_trial_offer` escrows a fee; `confirm_trial_offer`
# is the only place that calls `progress.advance_level`). Docs previously
# described `log_trial_offer` as advancing the level directly, which stopped
# being true once the escrow-based flow shipped.
#
# This script re-derives the ground truth from
# contracts/scout_access/src/lib.rs (source, not docs) and asserts:
#
#   1. `log_trial_offer`'s function body does NOT call `advance_level`.
#   2. At least one `confirm_trial_offer` function body DOES call
#      `advance_level`.
#   3. README.md's Level 2 -> Level 3 transition row mentions
#      `confirm_trial_offer`, not just `log_trial_offer`.
#   4. docs/CONTRACT_REFERENCE.md has a dedicated `confirm_trial_offer` doc
#      entry (heading), and its `log_trial_offer` entry no longer claims to
#      call `advance_level`.
#
# Exit codes:
#   0 — docs and source agree
#   1 — a stale/incorrect claim was found
#
# Usage:
#   bash scripts/check-trial-offer-flow-consistency.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB_RS="$REPO_ROOT/contracts/scout_access/src/lib.rs"
README="$REPO_ROOT/README.md"
CONTRACT_REF="$REPO_ROOT/docs/CONTRACT_REFERENCE.md"

for f in "$LIB_RS" "$README" "$CONTRACT_REF"; do
  if [[ ! -f "$f" ]]; then
    echo "ERROR: $f not found. Run from the repository root." >&2
    exit 1
  fi
done

python3 - "$LIB_RS" "$README" "$CONTRACT_REF" <<'PYEOF'
import re, sys

lib_rs_path, readme_path, cref_path = sys.argv[1:4]
lib_src = open(lib_rs_path).read()
errors = 0

def function_bodies(src, fn_name):
    """Yield the full source text of every `pub fn <fn_name>(...) { ... }`
    body found in src, matched by brace-depth (handles overloaded/duplicate
    definitions of the same name)."""
    bodies = []
    for m in re.finditer(r'\bpub fn\s+' + re.escape(fn_name) + r'\s*\(', src):
        # Walk forward from the match to find the opening '{' of the body,
        # then track brace depth to find the matching '}'.
        i = m.end()
        depth = 0
        body_start = None
        while i < len(src):
            ch = src[i]
            if ch == '{':
                if body_start is None:
                    body_start = i
                depth += 1
            elif ch == '}':
                depth -= 1
                if depth == 0 and body_start is not None:
                    bodies.append(src[body_start:i + 1])
                    break
            i += 1
    return bodies

# ---------------------------------------------------------------------------
# 1. log_trial_offer must NOT call advance_level directly.
# ---------------------------------------------------------------------------
log_bodies = function_bodies(lib_src, "log_trial_offer")
if not log_bodies:
    print("FAIL: could not find `pub fn log_trial_offer` in contracts/scout_access/src/lib.rs")
    errors += 1
else:
    for body in log_bodies:
        if "advance_level" in body:
            print("FAIL: `log_trial_offer` body calls `advance_level` directly — "
                  "the two-step escrow flow expects only `confirm_trial_offer` to do this. "
                  "If this is an intentional contract change, update README.md, ai.md, and "
                  "docs/CONTRACT_REFERENCE.md's trial-offer flow description to match.")
            errors += 1
    if not any("TrialEscrow" in body for body in log_bodies):
        print("FAIL: `log_trial_offer` body no longer creates a `TrialEscrow` record — "
              "the documented two-step flow assumes it does.")
        errors += 1

# ---------------------------------------------------------------------------
# 2. At least one confirm_trial_offer body must call advance_level — this is
#    the function docs point to as the actual level-advancement trigger.
# ---------------------------------------------------------------------------
confirm_bodies = function_bodies(lib_src, "confirm_trial_offer")
if not confirm_bodies:
    print("FAIL: could not find `pub fn confirm_trial_offer` in contracts/scout_access/src/lib.rs")
    errors += 1
elif not any("advance_level" in body for body in confirm_bodies):
    print("FAIL: no `confirm_trial_offer` body calls `advance_level` — docs describe this as "
          "the function that advances the player to Level 3. If level advancement moved "
          "elsewhere, update README.md, ai.md, and docs/CONTRACT_REFERENCE.md to match.")
    errors += 1

# ---------------------------------------------------------------------------
# 3. README's Level 2 -> Level 3 transition row must mention confirm_trial_offer.
# ---------------------------------------------------------------------------
readme_src = open(readme_path).read()
m = re.search(r'\|\s*Level 2\s*\|\s*Level 3\s*\|(.*)\|', readme_src)
if not m:
    print("FAIL: could not find the 'Level 2 | Level 3' row in README.md's "
          "'Valid Transitions' table.")
    errors += 1
elif "confirm_trial_offer" not in m.group(1):
    print("FAIL: README.md's Level 2 -> Level 3 transition row does not mention "
          "`confirm_trial_offer` — it must describe the two-step "
          "log_trial_offer -> confirm_trial_offer flow, not log_trial_offer alone.")
    errors += 1

# Sequence diagram must show a player-initiated confirm_trial_offer call, not
# an immediate Level-3 update straight out of log_trial_offer.
seq_section_m = re.search(
    r'Sequence Diagram\s*```mermaid(.*?)```', readme_src, re.DOTALL)
if not seq_section_m:
    print("FAIL: could not find the Player Lifecycle sequence diagram "
          "(```mermaid fenced block after a 'Sequence Diagram' heading) in README.md.")
    errors += 1
elif "confirm_trial_offer" not in seq_section_m.group(1):
    print("FAIL: README.md's Player Lifecycle sequence diagram does not mention "
          "`confirm_trial_offer` in the trial-offer section.")
    errors += 1

# ---------------------------------------------------------------------------
# 4. CONTRACT_REFERENCE.md must have a dedicated confirm_trial_offer entry,
#    and log_trial_offer's entry must not claim to call advance_level.
# ---------------------------------------------------------------------------
cref_src = open(cref_path).read()
if not re.search(r'####\s+`confirm_trial_offer\(', cref_src):
    print("FAIL: docs/CONTRACT_REFERENCE.md has no `#### `confirm_trial_offer(...)`` "
          "heading — a dedicated check-precedence entry is required.")
    errors += 1

log_entry_m = re.search(
    r'####\s+`log_trial_offer\(.*?(?=\n#### |\Z)', cref_src, re.DOTALL)
if not log_entry_m:
    print("FAIL: docs/CONTRACT_REFERENCE.md has no `log_trial_offer` entry.")
    errors += 1
else:
    entry = log_entry_m.group(0)
    if re.search(r'Also calls\s*`?progress\.advance_level', entry, re.IGNORECASE):
        print("FAIL: docs/CONTRACT_REFERENCE.md's `log_trial_offer` entry still claims "
              "it calls `progress.advance_level` — that call now lives in "
              "`confirm_trial_offer` only.")
        errors += 1
    if not re.search(r'does\s+\*\*not\*\*\s+call\s+`?progress\.advance_level',
                      entry, re.IGNORECASE):
        print("FAIL: docs/CONTRACT_REFERENCE.md's `log_trial_offer` entry no longer "
              "explicitly states that it does not call `progress.advance_level` — "
              "an SDK integrator reading only this entry could assume it advances "
              "the level directly.")
        errors += 1
    if re.search(r'\|\s*ProgressCallFailed\s*\(14\)\s*\|', entry):
        print("FAIL: docs/CONTRACT_REFERENCE.md's `log_trial_offer` check-precedence "
              "table still has a ProgressCallFailed row; that error can no longer "
              "originate from log_trial_offer.")
        errors += 1

if errors == 0:
    print("OK: trial-offer flow docs (README.md, docs/CONTRACT_REFERENCE.md) agree with "
          "contracts/scout_access/src/lib.rs — log_trial_offer escrows only, "
          "confirm_trial_offer is the sole caller of advance_level.")
    sys.exit(0)
else:
    print(f"\nFAIL: {errors} trial-offer flow doc/source mismatch(es) found.")
    sys.exit(1)
PYEOF
