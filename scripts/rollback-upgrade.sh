#!/usr/bin/env bash
# Revert an in-place contract upgrade by re-upgrading to the previously
# deployed WASM hash recorded by upgrade.sh (#1812).
#
# Usage: ./scripts/rollback-upgrade.sh <registration|verification|progress|scout_access|all>
#          [--network <net>] [--source <identity>] [--tag <release>]
#
# --source defaults to $UPGRADE_SOURCE. The previous WASM must still be
# installed on the network (installed code is not removed by upgrades).
# WARNING: storage migrations done by the new WASM (migrate()) are NOT reverted;
# see docs/DEPLOYMENT.md before rolling back across a schema_version bump.
set -euo pipefail

TARGET="${1:-}"; shift || true
NETWORK="testnet"; SOURCE="${UPGRADE_SOURCE:-}"; TAG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --network) NETWORK="${2:-}"; shift 2 ;;
    --source)  SOURCE="${2:-}"; shift 2 ;;
    --tag)     TAG="${2:-}"; shift 2 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done
if [[ -z "$TARGET" || -z "$SOURCE" ]]; then
  echo "usage: rollback-upgrade.sh <contract|all> --source <identity> [--network <net>] [--tag <release>]" >&2
  exit 1
fi
[[ -f .env.contracts ]] || { echo "ERROR: .env.contracts not found" >&2; exit 1; }

# shellcheck disable=SC1091
source "$(dirname "${BASH_SOURCE[0]}")/lib/wasm-hash.sh"

if [[ "$TARGET" == "all" ]]; then NAMES=(REGISTRATION VERIFICATION PROGRESS SCOUT_ACCESS)
else NAMES=("$(echo "$TARGET" | tr '[:lower:]' '[:upper:]')"); fi

cp .env.contracts .env.contracts.pre-rollback
fail=0
for name in "${NAMES[@]}"; do
  id="$(grep -E "^${name}_CONTRACT_ID=" .env.contracts | head -1 | cut -d= -f2- | tr -d "\"'")"
  prev="$(grep -E "^${name}_PREVIOUS_WASM_HASH=" .env.contracts | head -1 | cut -d= -f2- | tr -d "\"'")"
  if [[ -z "$id" || -z "$prev" ]]; then
    echo "ERROR: no ${name}_CONTRACT_ID / ${name}_PREVIOUS_WASM_HASH in .env.contracts (was upgrade.sh run?)" >&2
    fail=1; continue
  fi
  echo "==> $name: upgrading $id back to $prev"
  stellar contract invoke --id "$id" --network "$NETWORK" --source "$SOURCE" -- upgrade "$prev"
  now="$(onchain_wasm_hash "$id" "$NETWORK" || true)"
  if [[ "$now" == "$prev" ]]; then
    echo "    verified: on-chain hash matches $prev"
  else
    echo "ERROR: $name on-chain hash is '${now:-unknown}', expected $prev" >&2
    fail=1
  fi
done

if [[ -n "$TAG" ]]; then
  bash "$(dirname "$0")/verify-deployed-wasm.sh" "$NETWORK" "$TAG" || fail=1
fi
if [[ $fail -ne 0 ]]; then echo "Rollback of upgrade FAILED." >&2; exit 1; fi
echo "==> Upgrade rollback successful."
