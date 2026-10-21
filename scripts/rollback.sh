#!/usr/bin/env bash
# ScoutChain — rollback to the last known good contract addresses
# Restores .env.contracts from the snapshot saved by deploy.sh before the last deployment.
# Usage: ./scripts/rollback.sh [testnet|mainnet|local]
set -euo pipefail

NETWORK="${1:-testnet}"

SNAPSHOT=".env.contracts.snapshot"

if [[ ! -f "$SNAPSHOT" ]]; then
  echo "ERROR: No snapshot found at $SNAPSHOT"
  echo "       A snapshot is only created when deploy.sh runs and .env.contracts already exists."
  echo "       Manual recovery: restore .env.contracts from your source-control backup or"
  echo "       re-deploy from scratch with ./scripts/deploy.sh $NETWORK"
  exit 1
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
