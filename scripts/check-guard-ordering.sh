#!/usr/bin/env bash
# check-guard-ordering.sh
#
# Audits every state-changing pub fn in all four Soroban contract source files
# and asserts that:
#
#   1. If a function calls BOTH require_not_paused AND require_initialized,
#      require_not_paused must appear BEFORE require_initialized.
#
#   2. Every state-changing function that calls require_not_paused must also
#      call require_initialized (unless it is an admin-only function, a pure
#      getter, or otherwise intentionally exempt — exemptions are listed below).
#
# Exit codes:
#   0  — all checks pass
#   1  — one or more violations found
#
# Usage:
#   bash scripts/check-guard-ordering.sh
#
# Run automatically by the CI lint job in .github/workflows/contract-ci.yml.
# Run locally before opening a PR with:
#   bash scripts/check-guard-ordering.sh

set -euo pipefail

CONTRACTS=(
    contracts/registration/src/lib.rs
    contracts/verification/src/lib.rs
    contracts/progress/src/lib.rs
    contracts/scout_access/src/lib.rs
)

# Functions that intentionally skip require_initialized (or the full pair)
# because they are read-only queries or admin-only bootstrap operations.
# Each entry is the bare function name.
INTENTIONAL_EXEMPTIONS=(
    # filter_players — read-only query; intentionally skips paused check too.
    "filter_players"
    # version / health — pure getters in every contract.
    "version"
    "health"
    # transfer_admin / upgrade — admin-only; require_admin is sufficient.
    "transfer_admin"
    "upgrade"
    # update_progress_contract — admin-only re-wiring, no user data access.
    "update_progress_contract"
)

ERRORS=0

is_exempt() {
    local fn_name="$1"
    for exempt in "${INTENTIONAL_EXEMPTIONS[@]}"; do
        if [[ "$exempt" == "$fn_name" ]]; then
            return 0
        fi
    done
    return 1
}

is_getter() {
    local fn_name="$1"
    # Pure getter heuristic: name starts with one of these prefixes.
    if [[ "$fn_name" =~ ^(get_|has_|is_|list_|filter_|version$|health$) ]]; then
        return 0
    fi
    return 1
}

for file in "${CONTRACTS[@]}"; do
    if [[ ! -f "$file" ]]; then
        echo "ERROR: source file not found: $file" >&2
        ERRORS=$((ERRORS + 1))
        continue
    fi

    mapfile -t lines < "$file"
    total=${#lines[@]}
    contract_name=$(basename "$(dirname "$(dirname "$file")")")

    i=0
    while [[ $i -lt $total ]]; do
        line="${lines[$i]}"

        # Match a pub fn declaration inside a contract impl block.
        if [[ "$line" =~ ^[[:space:]]+pub[[:space:]]+fn[[:space:]]+([a-zA-Z_][a-zA-Z0-9_]*)[[:space:]]*\( ]]; then
            fn_name="${BASH_REMATCH[1]}"
            line_num=$((i + 1))

            # Skip private helpers (they start with `fn`, not `pub fn` — but
            # our regex already requires pub fn, so this is fine).

            # Skip getters and exempt functions early.
            if is_getter "$fn_name" || is_exempt "$fn_name"; then
                i=$((i + 1))
                continue
            fi

            # Scan forward to collect just the first STATEMENT block of the
            # function body (stop after we've seen the opening `{` and
            # collected up to 20 lines, or until a line that is another
            # `pub fn` declaration).
            has_not_paused=false
            has_initialized=false
            np_line=-1
            ri_line=-1

            in_body=false
            scan_count=0
            j=$i

            while [[ $j -lt $total && $scan_count -lt 30 ]]; do
                wline="${lines[$j]}"

                # Detect we've entered the function body
                if [[ "$wline" == *"{"* ]]; then
                    in_body=true
                fi

                # Stop if we hit the next pub fn (we've left this function's
                # opening guard section).
                if [[ $j -gt $i && "$wline" =~ ^[[:space:]]+pub[[:space:]]+fn[[:space:]] ]]; then
                    break
                fi

                if $in_body; then
                    if [[ "$wline" == *"require_not_paused"* ]]; then
                        has_not_paused=true
                        [[ $np_line -eq -1 ]] && np_line=$scan_count
                    fi
                    if [[ "$wline" == *"require_initialized"* ]]; then
                        has_initialized=true
                        [[ $ri_line -eq -1 ]] && ri_line=$scan_count
                    fi
                    # Stop scanning after 12 lines into the body — guards are
                    # always in the first few lines.
                    if [[ $scan_count -ge 12 ]]; then
                        break
                    fi
                    scan_count=$((scan_count + 1))
                fi

                j=$((j + 1))
            done

            # Check 1: wrong order
            if $has_not_paused && $has_initialized; then
                if [[ $ri_line -lt $np_line ]]; then
                    echo "FAIL [$contract_name] $fn_name (line $line_num): require_initialized appears BEFORE require_not_paused — expected not_paused → initialized"
                    ERRORS=$((ERRORS + 1))
                fi
            fi

            # Check 2: has not_paused but missing initialized
            if $has_not_paused && ! $has_initialized; then
                echo "FAIL [$contract_name] $fn_name (line $line_num): calls require_not_paused but is missing require_initialized"
                ERRORS=$((ERRORS + 1))
            fi
        fi

        i=$((i + 1))
    done
done

if [[ $ERRORS -eq 0 ]]; then
    echo "OK: guard-ordering check passed across all four contracts."
    exit 0
else
    echo ""
    echo "FAIL: $ERRORS guard-ordering violation(s) found."
    echo "Every state-changing pub fn must call require_not_paused before require_initialized."
    echo "If a deviation is intentional, add the function to the INTENTIONAL_EXEMPTIONS"
    echo "list inside scripts/check-guard-ordering.sh with a comment explaining why."
    exit 1
fi
