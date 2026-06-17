#!/usr/bin/env bash
# ScoutChain — full testnet setup in one command
# Runs: build → deploy → initialize → generate-bindings → seed
set -euo pipefail

echo "========================================"
echo "  ScoutChain Testnet Setup"
echo "========================================"

if [[ ! -f .env ]]; then
  echo "ERROR: .env not found. Copy .env.example to .env and fill in values."
  exit 1
fi

source .env

echo ""
echo "Step 1/5 — Build contracts"
cargo build --workspace --target wasm32v1-none --release

echo ""
echo "Step 2/5 — Deploy contracts"
chmod +x scripts/deploy.sh
./scripts/deploy.sh testnet

echo ""
echo "Step 3/5 — Initialize contracts"
chmod +x scripts/initialize.sh
./scripts/initialize.sh testnet

echo ""
echo "Step 4/5 — Generate TypeScript bindings"
chmod +x scripts/generate-bindings.sh
./scripts/generate-bindings.sh testnet

echo ""
echo "Step 5/5 — Seed demo data"
chmod +x testnet/seed.sh
./testnet/seed.sh

echo ""
echo "========================================"
echo "  Setup complete!"
echo "  Contract IDs: .env.contracts"
echo "  Test accounts: testnet/.accounts"
echo "  Bindings:      bindings/"
echo "========================================"
