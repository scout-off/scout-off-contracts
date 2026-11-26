#!/usr/bin/env bash
# ScoutChain — tear down the local development stack.
#
# Usage: ./scripts/dev-down.sh [--volumes]
#
# Stops and removes all containers started by dev-up.sh.
# Pass --volumes to also delete persistent volumes (Postgres data).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

VOLUMES_FLAG=""
for arg in "$@"; do
  case "$arg" in
    --volumes|-v) VOLUMES_FLAG="--volumes" ;;
    --help|-h)
      echo "Usage: $0 [--volumes]"
      echo "  --volumes  Also remove Docker volumes (erases Postgres data)"
      exit 0 ;;
  esac
done

echo "==> Stopping ScoutChain local stack..."
# shellcheck disable=SC2086
docker compose down $VOLUMES_FLAG

if [ -n "$VOLUMES_FLAG" ]; then
  echo "==> Removing generated local env files..."
  rm -f "$REPO_ROOT/.env.local" "$REPO_ROOT/.env.contracts"
fi

echo ""
echo "Stack stopped."
[ -n "$VOLUMES_FLAG" ] && echo "Volumes deleted. Next dev-up.sh will start fresh." \
  || echo "Postgres data preserved. Use --volumes to wipe."
