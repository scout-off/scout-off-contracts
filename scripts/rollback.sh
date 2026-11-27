#!/usr/bin/env bash
# ScoutChain — rollback to the last known good contract addresses
# Restores .env.contracts from the snapshot saved by deploy.sh before the last deployment.
# Usage: ./scripts/rollback.sh [testnet|mainnet|local]
#        ./scripts/rollback.sh --list
#        ./scripts/rollback.sh --snapshot <file> [testnet|mainnet|local]
set -euo pipefail

# shellcheck source=scripts/network.sh
source "$(dirname "$0")/network.sh" "${1:-}"

SNAPSHOT=".env.contracts.snapshot"
SNAPSHOT_DIR=".env.contracts.snapshots"

usage() {
  cat <<EOF
Usage: ./scripts/rollback.sh [testnet|mainnet|local]
       ./scripts/rollback.sh --list
       ./scripts/rollback.sh --snapshot <file> [testnet|mainnet|local]

Options:
  --list                 List available snapshots in $SNAPSHOT_DIR/ (newest first)
  --snapshot <file>      Restore a specific snapshot file
  -h, --help             Show this help message

Without options, restores the latest snapshot ($SNAPSHOT, or the newest file
in $SNAPSHOT_DIR/ if present) and runs a health check.
EOF
}

list_snapshots() {
  if [[ ! -d "$SNAPSHOT_DIR" ]]; then
    echo "No snapshots found: $SNAPSHOT_DIR/ does not exist."
    return 0
  fi
  local files
  files=$(find "$SNAPSHOT_DIR" -maxdepth 1 -type f -name '*.env' -print 2>/dev/null | sort -r || true)
  if [[ -z "$files" ]]; then
    echo "No snapshots found in $SNAPSHOT_DIR/."
    return 0
  fi
  echo "Available snapshots in $SNAPSHOT_DIR/ (newest first):"
  echo "$files"
}

NETWORK="testnet"
SNAPSHOT_FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      list_snapshots
      exit 0
      ;;
    --snapshot)
      if [[ $# -lt 2 ]]; then
        echo "ERROR: --snapshot requires a file argument"
        usage
        exit 1
      fi
      SNAPSHOT_FILE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "ERROR: Unknown option: $1"
      usage
      exit 1
      ;;
    *)
      NETWORK="$1"
      shift
      ;;
  esac
done

if [[ -n "$SNAPSHOT_FILE" ]]; then
  if [[ ! -f "$SNAPSHOT_FILE" ]]; then
    echo "ERROR: Snapshot file not found: $SNAPSHOT_FILE"
    echo "       Use --list to see available snapshots."
    exit 1
  fi
  SNAPSHOT="$SNAPSHOT_FILE"
else
  # Default behaviour: prefer the newest timestamped snapshot if present,
  # otherwise fall back to the legacy .env.contracts.snapshot file.
  if [[ -d "$SNAPSHOT_DIR" ]]; then
    NEWEST=$(find "$SNAPSHOT_DIR" -maxdepth 1 -type f -name '*.env' -print 2>/dev/null | sort -r | head -n 1 || true)
    if [[ -n "$NEWEST" ]]; then
      SNAPSHOT="$NEWEST"
    fi
  fi

  if [[ ! -f "$SNAPSHOT" ]]; then
    echo "ERROR: No snapshot found at $SNAPSHOT"
    echo "       A snapshot is only created when deploy.sh runs and .env.contracts already exists."
    echo "       Use --list to see available snapshots."
    echo "       Manual recovery: restore .env.contracts from your source-control backup or"
    echo "       re-deploy from scratch with ./scripts/deploy.sh $NETWORK"
    exit 1
  fi
fi

if [[ -f .env.contracts ]]; then
  cp .env.contracts .env.contracts.pre-rollback
  echo "    Backed up current .env.contracts to .env.contracts.pre-rollback"
fi
echo "==> Rolling back .env.contracts to snapshot..."
cp "$SNAPSHOT" .env.contracts
echo "    Restored from $SNAPSHOT"

echo ""
echo "==> Snapshot contents:"
cat .env.contracts

echo ""
echo "==> Re-running health check on restored contract addresses..."
if bash scripts/health-check.sh "$NETWORK"; then
  echo ""
  echo "==> Rollback successful. All contracts from snapshot are healthy."
else
  echo ""
  echo "ERROR: Rollback completed but health check failed."
  echo "       The snapshot contracts may also be in a bad state."
  echo "       Check the contract addresses above and consider redeploying."
  exit 1
fi