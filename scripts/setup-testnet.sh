#!/usr/bin/env bash
# ScoutChain — full testnet setup in one command
# Runs: deploy → initialize → generate-bindings → seed
set -euo pipefail

echo "========================================"
echo "  ScoutChain Testnet Setup"
echo "========================================"

if [[ ! -f .env ]]; then
  echo "ERROR: .env not found. Copy .env.example to .env and fill in values."
  exit 1
fi

# shellcheck disable=SC1091
source .env

echo ""
echo "Step 1/4 — Deploy contracts"
chmod +x scripts/deploy.sh
./scripts/deploy.sh testnet

echo ""
echo "Step 2/4 — Initialize contracts"
chmod +x scripts/initialize.sh
./scripts/initialize.sh testnet

echo ""
echo "Step 3/4 — Generate TypeScript bindings"
chmod +x scripts/generate-bindings.sh
./scripts/generate-bindings.sh testnet

echo ""
echo "Step 4/4 — Seed demo data"
chmod +x testnet/seed.sh
./testnet/seed.sh

echo ""
echo "========================================"
echo "  Setup complete!"
echo "  Contract IDs: .env.contracts"
echo "  Test accounts: testnet/.accounts"
echo "  Bindings:      bindings/"
echo "========================================"