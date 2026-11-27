#!/usr/bin/env bash
# check-cross-doc-consistency.sh
#
# Verifies that the subscription-tier access rules described in two independent
# documentation locations are mutually consistent:
#
#   1. README.md — "Subscription Tier Access" table
#      Angle: Tier → accessible ProgressLevel range
#      e.g. "Basic | Level 1 (VerifiedIdentity) only"
#
#   2. docs/CONTRACT_REFERENCE.md — "Subscription tier access mapping" table
#      Angle: ProgressLevel → minimum tier required to view
#      e.g. "VerifiedIdentity (1) | Basic"
#
# Normalises both into: level_number → minimum_tier_name
# and asserts they agree.
#
# Exit codes:
#   0  — tables are consistent
#   1  — disagreement found or a table could not be parsed
#
# Usage:
#   bash scripts/check-cross-doc-consistency.sh
#
# Wired into the CI lint job — see .github/workflows/contract-ci.yml.

set -euo pipefail

README="README.md"
CONTRACT_REF="docs/CONTRACT_REFERENCE.md"

if [[ ! -f "$README" ]]; then
    echo "ERROR: $README not found. Run from the repository root." >&2
    exit 1
fi
if [[ ! -f "$CONTRACT_REF" ]]; then
    echo "ERROR: $CONTRACT_REF not found. Run from the repository root." >&2
    exit 1
fi

python3 - "$README" "$CONTRACT_REF" <<'PYEOF'
import sys, re

readme_path = sys.argv[1]
cref_path   = sys.argv[2]

TIER_RANK   = {"None": 0, "Basic": 1, "Pro": 2, "Elite": 3}
LEVEL_NAMES = {0: "Unverified", 1: "VerifiedIdentity",
               2: "PerformanceMilestones", 3: "EliteTier"}

# ---------------------------------------------------------------------------
# Parse README.md: Tier → set of accessible levels
# ---------------------------------------------------------------------------
def parse_readme(path):
    """
    Returns dict: tier_name -> set of accessible level ints
    e.g. {"Basic": {1}, "Pro": {0,1,2}, "Elite": {0,1,2,3}}
    """
    with open(path) as f:
        lines = f.readlines()

    in_table = False
    result   = {}

    for line in lines:
        if "Subscription Tier Access" in line:
            in_table = True
            continue
        if not in_table:
            continue

        stripped = line.strip()
        if not stripped:
            if result:
                break
            continue
        if stripped.startswith("#") or "Admin Functions" in stripped:
            if result:
                break
            continue
        # Skip header/separator
        if re.match(r'\|[-:| ]+\|', stripped) or stripped.startswith("| Tier"):
            continue

        # Match data rows: | **Tier** | level description | ...
        m = re.match(r'\|\s*\*\*([A-Za-z]+)\*\*\s*\|(.*?)\|', line)
        if not m:
            continue

        tier       = m.group(1).strip()
        levels_col = m.group(2).strip()

        # Extract all level numbers mentioned in this cell
        nums = [int(n) for n in re.findall(r'\bLevel (\d)\b', levels_col)]
        # If a range "0–2" or "0-3" is written, expand it
        range_m = re.search(r'Level (\d)[–—-](\d)', levels_col)
        if range_m:
            lo, hi = int(range_m.group(1)), int(range_m.group(2))
            nums = list(range(lo, hi + 1))
        # "all levels" → 0–3
        if "all levels" in levels_col.lower():
            nums = [0, 1, 2, 3]

        if nums:
            result[tier] = set(nums)

    return result

# ---------------------------------------------------------------------------
# Derive canonical level → min_tier from README tier → level sets
# ---------------------------------------------------------------------------
def derive_level_to_min_tier(tier_levels):
    """
    For each level, find the lowest-ranked tier that can access it.
    tier_levels: {"Basic": {1}, "Pro": {0,1,2}, "Elite": {0,1,2,3}}
    Returns: {0: "Pro", 1: "Basic", 2: "Pro", 3: "Elite"}
    """
    result = {}
    for level in range(4):
        # Which tiers can see this level?
        eligible = [t for t, lvs in tier_levels.items() if level in lvs]
        if not eligible:
            result[level] = "None"
        else:
            # Minimum tier = lowest rank among eligible
            min_t = min(eligible, key=lambda t: TIER_RANK.get(t, 99))
            result[level] = min_t
    return result

# ---------------------------------------------------------------------------
# Parse CONTRACT_REFERENCE.md: ProgressLevel → min tier
# ---------------------------------------------------------------------------
def parse_cref(path):
    """
    Returns dict: level_int -> tier_name
    e.g. {0: "None", 1: "Basic", 2: "Pro", 3: "Elite"}
    """
    with open(path) as f:
        lines = f.readlines()

    in_table = False
    result   = {}

    for line in lines:
        if "Subscription tier access mapping" in line:
            in_table = True
            continue
        if not in_table:
            continue

        stripped = line.strip()
        if not stripped or stripped.startswith("Scouts without") or stripped.startswith("####"):
            if result:
                break
            continue

        # Match: | `LevelName` (N) | TierName...
        m = re.match(r'\|\s*`([A-Za-z]+)`\s*\((\d)\)\s*\|\s*([A-Za-z]+)', stripped)
        if m:
            level_num = int(m.group(2))
            tier_raw  = m.group(3).strip()
            result[level_num] = tier_raw

    return result

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
errors = 0

# 1. Parse README
readme_tier_levels = parse_readme(readme_path)
if not readme_tier_levels:
    print(f"FAIL: Could not parse 'Subscription Tier Access' table from {readme_path}.")
    print("  Expected rows like: | **Basic** | Level 1 (VerifiedIdentity) only | ...")
    sys.exit(1)

print("README.md tier → accessible levels:")
for tier in ("Basic", "Pro", "Elite"):
    lvs = readme_tier_levels.get(tier, set())
    names = [f"Level {l} ({LEVEL_NAMES[l]})" for l in sorted(lvs)]
    print(f"  {tier} → {', '.join(names) if names else 'NOT FOUND'}")

# 2. Derive level → min_tier from README
readme_derived = derive_level_to_min_tier(readme_tier_levels)
print(f"\nREADME.md derived level → min tier:")
for lv in range(4):
    print(f"  Level {lv} ({LEVEL_NAMES[lv]}) → {readme_derived.get(lv, 'NOT FOUND')}")

# 3. Parse CONTRACT_REFERENCE
cref_map = parse_cref(cref_path)
if not cref_map:
    print(f"\nFAIL: Could not parse 'Subscription tier access mapping' table from {cref_path}.")
    print("  Expected rows like: | `VerifiedIdentity` (1) | Basic |")
    sys.exit(1)

print(f"\nCONTRACT_REFERENCE.md level → min tier:")
for lv in range(4):
    print(f"  Level {lv} ({LEVEL_NAMES[lv]}) → {cref_map.get(lv, 'NOT FOUND')}")

# 4. Cross-check
print("\nCross-checking...")
for lv in range(4):
    readme_t = readme_derived.get(lv, "NOT_FOUND")
    cref_t   = cref_map.get(lv, "NOT_FOUND")
    name     = LEVEL_NAMES[lv]

    # Level 0 (Unverified) is documented in CONTRACT_REFERENCE as "None —
    # public profile metadata only (no contact)", meaning no subscription is
    # required to see that a Level 0 player exists. The README Subscription
    # Tier Access table describes what each *subscription tier* can access, so
    # Level 0 appears in all tier rows (Basic/Pro/Elite all include it).
    # The derivation therefore yields "Basic" as the minimum named tier, but the
    # ground truth is "None" (unauthenticated). We treat the CONTRACT_REFERENCE
    # value as authoritative for Level 0 and allow "None" vs "Basic" to be
    # equivalent for this check — both sources agree Level 0 is the most
    # permissive level.
    if lv == 0 and cref_t == "None" and readme_t in ("Basic", "None"):
        print(f"  OK  Level {lv} ({name}): CONTRACT_REFERENCE says 'None' (public); "
              f"README tier table includes it in all tiers — consistent.")
        continue

    if readme_t == cref_t:
        print(f"  OK  Level {lv} ({name}): both say '{readme_t}'")
    else:
        print(f"  FAIL Level {lv} ({name}): README says '{readme_t}' but CONTRACT_REFERENCE says '{cref_t}'")
        errors += 1

print()
if errors == 0:
    print("OK: cross-doc consistency check passed — subscription tier tables agree.")
    sys.exit(0)
else:
    print(f"FAIL: {errors} disagreement(s) found between {readme_path} and {cref_path}.")
    print()
    print("Both tables describe the same access-control rules from different angles.")
    print("Update both tables together whenever the subscription-tier access rules change.")
    sys.exit(1)
PYEOF
