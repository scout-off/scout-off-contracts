#!/usr/bin/env bash
set -euo pipefail

# shellcheck disable=SC1091
source "$(dirname "$0")/../.env"
# shellcheck disable=SC1091
source "$(dirname "$0")/../.env.contracts"

NETWORK_ARGS=("--network" "$STELLAR_NETWORK" "--source" "$ADMIN_SECRET")

usage() {
  cat <<'EOF'
Usage: emergency-pause.sh [--function approve_milestone|pay_to_contact]

Without --function, pauses all four contracts (whole-contract pause).

With --function, engages only the corresponding function-scoped circuit breaker:
  approve_milestone  -> verification.pause_approve_milestone
  pay_to_contact     -> scout_access.pause_pay_to_contact
EOF
}

FUNCTION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --function)
      if [[ $# -lt 2 ]]; then
        echo "Error: --function requires a value (approve_milestone|pay_to_contact)." >&2
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

if [[ -n "$FUNCTION" ]]; then
  case "$FUNCTION" in
    approve_milestone)
      echo "==> Pausing verification.pause_approve_milestone..."
      stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
        -- pause_approve_milestone

      echo "==> verification.health()..."
      stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
        -- health
      ;;
    pay_to_contact)
      echo "==> Pausing scout_access.pause_pay_to_contact..."
      stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
        -- pause_pay_to_contact

      echo "==> scout_access.health()..."
      stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
        -- health
      ;;
    *)
      echo "Error: unknown --function value: $FUNCTION (expected approve_milestone|pay_to_contact)." >&2
      usage >&2
      exit 1
      ;;
  esac

  echo "Function-scoped breaker engaged: $FUNCTION"
  exit 0
fi

echo "==> Pausing registration contract..."
stellar contract invoke --id "$REGISTRATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "==> Pausing verification contract..."
stellar contract invoke --id "$VERIFICATION_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "==> Pausing progress contract..."
stellar contract invoke --id "$PROGRESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "==> Pausing scout_access contract..."
stellar contract invoke --id "$SCOUT_ACCESS_CONTRACT_ID" "${NETWORK_ARGS[@]}" \
  -- pause_contract

echo "All four contracts paused."