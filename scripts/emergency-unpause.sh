#!/usr/bin/env bash
set -euo pipefail

# Emergency unpause script - best-effort across all contracts
# Unpauses contracts in order of risk: scout_access (funds) first, then the rest.
# Retries transient failures and verifies final state via health() calls.

# shellcheck disable=SC1091
source "$(dirname "$0")/network.sh"
# shellcheck disable=SC1091
source "$(dirname "$0")/../.env"
# shellcheck disable=SC1091
source "$(dirname "$0")/../.env.contracts"

# Configuration
MAX_RETRIES=5
INITIAL_BACKOFF_MS=500
MAX_BACKOFF_MS=5000

# Track results
declare -A UNPAUSE_RESULTS
declare -A HEALTH_RESULTS
declare -a CONTRACTS_TO_UNPAUSE

# Parse optional --contracts flag
FILTER_CONTRACTS=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --contracts)
      FILTER_CONTRACTS="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# Determine which contracts to unpause
if [[ -n "$FILTER_CONTRACTS" ]]; then
  IFS=',' read -ra CONTRACTS_TO_UNPAUSE <<< "$FILTER_CONTRACTS"
  # Validate
  for c in "${CONTRACTS_TO_UNPAUSE[@]}"; do
    case "$c" in
      registration|verification|progress|scout_access) ;;
      *)
        echo "ERROR: Unknown contract '$c'. Valid: registration, verification, progress, scout_access" >&2
        exit 1
        ;;
    esac
  done
else
  CONTRACTS_TO_UNPAUSE=("registration" "verification" "progress" "scout_access")
fi

NETWORK_ARGS=("--network" "$STELLAR_NETWORK" "--source" "$ADMIN_SECRET")

# Helper: exponential backoff with jitter
wait_with_backoff() {
  local attempt=$1
  local backoff_ms=$((INITIAL_BACKOFF_MS * (2 ** attempt)))
  if [[ $backoff_ms -gt $MAX_BACKOFF_MS ]]; then
    backoff_ms=$MAX_BACKOFF_MS
  fi
  # Add 10% jitter
  local jitter=$((backoff_ms / 10))
  local sleep_ms=$((backoff_ms + RANDOM % jitter))
  echo "  Retrying in ${sleep_ms}ms..." >&2
  sleep "$(echo "scale=3; $sleep_ms / 1000" | bc)"
}

# Helper: check if contract is unpaused
is_unpaused() {
  local contract_id=$1
  local result
  result=$(stellar contract invoke --id "$contract_id" "${NETWORK_ARGS[@]}" -- health 2>/dev/null || echo '{"paused":null}')
  echo "$result" | grep -q '"paused":false' && echo "true" || echo "false"
}

# Helper: unpause a single contract with retries
unpause_contract_with_retry() {
  local name=$1
  local contract_id=$2

  local attempt=0
  while [[ $attempt -lt $MAX_RETRIES ]]; do
    if [[ $attempt -gt 0 ]]; then
      wait_with_backoff $attempt
    fi

    echo "  [attempt $((attempt + 1))/$MAX_RETRIES] Unpausing $name..."

    if stellar contract invoke --id "$contract_id" "${NETWORK_ARGS[@]}" -- unpause_contract >/dev/null 2>&1; then
      echo "  ✓ $name unpaused successfully"
      UNPAUSE_RESULTS[$name]="success"
      return 0
    fi

    local exit_code=$?
    # Check for error types that indicate we should NOT retry
    if [[ $exit_code -eq 127 ]]; then
      # Command not found - abort
      echo "  ✗ stellar CLI not found" >&2
      UNPAUSE_RESULTS[$name]="fail"
      return 1
    fi

    echo "  ✗ Attempt $((attempt + 1)) failed (exit: $exit_code)" >&2
    ((attempt++))
  done

  # All retries exhausted
  echo "  ✗ $name failed to unpause after $MAX_RETRIES attempts" >&2
  UNPAUSE_RESULTS[$name]="fail"
  return 1
}

echo "========================================"
echo "  Emergency Unpause - Starting"
echo "========================================"
echo ""
echo "Target network: $STELLAR_NETWORK"
echo "Contracts to unpause: ${CONTRACTS_TO_UNPAUSE[*]}"
echo ""

# Priority 1: scout_access (funds at risk) - unpause FIRST
if [[ " ${CONTRACTS_TO_UNPAUSE[*]} " =~ " scout_access " ]]; then
  echo "--- Priority: scout_access (funds at risk) ---"
  unpause_contract_with_retry "scout_access" "$SCOUT_ACCESS_CONTRACT_ID"
  echo ""
fi

# Priority 2: registration, verification, progress
for name in "${CONTRACTS_TO_UNPAUSE[@]}"; do
  if [[ "$name" == "scout_access" ]]; then
    continue  # Already handled above
  fi

  case "$name" in
    registration)
      unpause_contract_with_retry "registration" "$REGISTRATION_CONTRACT_ID"
      ;;
    verification)
      unpause_contract_with_retry "verification" "$VERIFICATION_CONTRACT_ID"
      ;;
    progress)
      unpause_contract_with_retry "progress" "$PROGRESS_CONTRACT_ID"
      ;;
  esac
  echo ""
done

echo "--- Verifying final state via health() ---"
echo ""

# Verify each contract is unpaused
for name in "${CONTRACTS_TO_UNPAUSE[@]}"; do
  contract_id_var="${name^^}_CONTRACT_ID"
  contract_id="${!contract_id_var}"

  health_result=""
  if health_result=$(stellar contract invoke --id "$contract_id" "${NETWORK_ARGS[@]}" -- health 2>/dev/null); then
    echo "$health_result"
    if echo "$health_result" | grep -q '"paused":false'; then
      HEALTH_RESULTS[$name]="paused:false"
    else
      HEALTH_RESULTS[$name]="paused:true"
    fi
  else
    echo "  ERROR: health() call failed for $name"
    HEALTH_RESULTS[$name]="health_failed"
  fi
done

echo ""
echo "========================================"
echo "  Emergency Unpause - Summary"
echo "========================================"
echo ""

# Print results table
printf "%-20s %-15s %-15s\n" "Contract" "Unpause Result" "Health Check"
printf "%-20s %-15s %-15s\n" "--------" "--------------" "------------"

all_unpaused=true
for name in "${CONTRACTS_TO_UNPAUSE[@]}"; do
  result="${UNPAUSE_RESULTS[$name]:-not_attempted}"
  health="${HEALTH_RESULTS[$name]:-not_checked}"
  printf "%-20s %-15s %-15s\n" "$name" "$result" "$health"

  if [[ "$result" != "success" || "$health" != "paused:false" ]]; then
    all_unpaused=false
  fi
done

echo ""
if $all_unpaused; then
  echo "✓ All targeted contracts are in UNPAUSED state"
  exit 0
else
  echo "✗ Some contracts are not in UNPAUSED state (see summary above)" >&2
  exit 1
fi
