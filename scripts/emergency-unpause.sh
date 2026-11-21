#!/usr/bin/env bash
set -euo pipefail

# shellcheck disable=SC1091
source "$(dirname "$0")/../.env"
# shellcheck disable=SC1091
source "$(dirname "$0")/../.env.contracts"

NETWORK_ARGS=("--network" "$STELLAR_NETWORK" "--source" "$ADMIN_SECRET")

usage() {
  cat <<'EOF'
Usage: emergency-unpause.sh [--function approve_milestone|pay_to_contact]

Without --function, unpauses all four contracts (whole-contract unpause).
With --function, unpauses only the specified function-scoped circuit breaker:
  approve_milestone  -> verification.unpause_approve_milestone
  pay_to_contact     -> scout_access.unpause_pay_to_contact
EOF
}

FUNCTION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --function)
      if [[ $# -lt 2 ]]; then
        echo "Error: --function requires a value" >&2
        usage >&2
        exit 1
      fi
      FUNCTION="$2"
      shift 2
      ;;
    --function=*)
      FUNCTION="${1#--function=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Error: unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

print_health() {
  local contract_id="$1"
  echo "==> health() for $contract_id:"
  stellar contract invoke --id "$contract_id" "${NETWORK_ARGS[@]}" \
    -- health
}

if [[ -n "$FUNCTION" ]]; then
  case "$FUNCTION" in
    approve_milestone)
      echo "==> Unpausing verification.pause_approve_milestone..."
      stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
        -- unpause_approve_milestone
      print_health "$VERIFICATION_CONTRACT_ID"
      ;;
    pay_to_contact)
      echo "==> Unpausing scout_access.pause_pay_to_contact..."
      stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
        -- unpause_pay_to_contact
      print_health "$SCOUT_ACCESS_CONTRACT_ID"
      ;;
    *)
      echo "Error: unknown --function value: $FUNCTION" >&2
      usage >&2
      exit 1
      ;;
  esac
  echo "Function-scoped breaker '$FUNCTION' unpaused."
  exit 0
fi

echo "==> Unpausing registration contract..."
stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- unpause_contract

echo "==> Unpausing verification contract..."
stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- unpause_contract

echo "==> Unpausing progress contract..."
stellar contract invoke --id "$PROGRESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- unpause_contract

echo "==> Unpausing scout_access contract..."
stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- unpause_contract

echo "All four contracts unpaused."
print_health "$REGISTRATION_CONTRACT_ID"
print_health "$VERIFICATION_CONTRACT_ID"
print_health "$PROGRESS_CONTRACT_ID"
print_health "$SCOUT_ACCESS_CONTRACT_ID"