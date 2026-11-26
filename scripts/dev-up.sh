#!/usr/bin/env bash
# ScoutChain — one-command local development stack bootstrap.
#
# Usage: ./scripts/dev-up.sh [--skip-build] [--skip-deploy]
#
# What this does:
#   1. Starts stellar/quickstart (local sandbox) + PostgreSQL via docker compose.
#   2. Builds WASM contracts (unless --skip-build).
#   3. Creates a local keypair, funds it via friendbot.
#   4. Deploys and initialises all four contracts to the local sandbox.
#   5. Wires cross-contract references.
#   6. Applies SQL migrations to Postgres.
#   7. Writes .env.local consumed by the backend container.
#   8. Starts the backend container.
#   9. Runs generate-bindings.sh so the TypeScript clients reflect the local
#      contract IDs.
#
# After this script exits successfully, everything is running:
#   • Soroban RPC:   http://localhost:8000/soroban/rpc
#   • Backend:       http://localhost:3000
#   • Postgres:      postgresql://scoutchain:scoutchain@localhost:5432/scoutchain_dev
#
# See docs/CONTRIBUTING.md § "Local development" for a full walkthrough.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Options ────────────────────────────────────────────────────────────────────
SKIP_BUILD=0
SKIP_DEPLOY=0
for arg in "$@"; do
  case "$arg" in
    --skip-build)  SKIP_BUILD=1 ;;
    --skip-deploy) SKIP_DEPLOY=1 ;;
    --help|-h)
      echo "Usage: $0 [--skip-build] [--skip-deploy]"
      echo "  --skip-build   Skip cargo wasm build (use pre-existing WASMs)"
      echo "  --skip-deploy  Skip contract deploy/init/wire (use existing .env.contracts)"
      exit 0 ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

# ── Constants ──────────────────────────────────────────────────────────────────
NETWORK="local"
NETWORK_PASSPHRASE="Standalone Network ; February 2017"
RPC_URL="http://localhost:8000/soroban/rpc"
WASM_DIR="$REPO_ROOT/target/wasm32v1-none/release"
KEY_NAME="local-deployer"
DB_URL="postgresql://scoutchain:scoutchain@localhost:5432/scoutchain_dev"

cd "$REPO_ROOT"

echo "╔══════════════════════════════════════════════════════╗"
echo "║   ScoutChain local stack — dev-up.sh                 ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

# ── 1. Start infrastructure services ──────────────────────────────────────────
echo "==> Starting stellar sandbox and postgres..."
docker compose up -d stellar postgres

echo "==> Waiting for Stellar RPC to be ready..."
for i in $(seq 1 60); do
  LEDGER=$(curl -sf "$RPC_URL" \
    -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' \
    2>/dev/null \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('result',{}).get('sequence',0))" \
    2>/dev/null || echo 0)
  if [ "${LEDGER:-0}" -gt 0 ] 2>/dev/null; then
    echo "   RPC ready (ledger ${LEDGER}) after ${i}×2 s"
    break
  fi
  sleep 2
done
if [ "${LEDGER:-0}" -eq 0 ] 2>/dev/null; then
  echo "ERROR: Soroban RPC never became ready. Check: docker compose logs stellar" >&2
  exit 1
fi

echo "==> Waiting for PostgreSQL to be ready..."
for i in $(seq 1 30); do
  if docker compose exec -T postgres pg_isready -U scoutchain -d scoutchain_dev >/dev/null 2>&1; then
    echo "   PostgreSQL ready after ${i}×2 s"
    break
  fi
  sleep 2
done

# ── 2. Build WASM ──────────────────────────────────────────────────────────────
if [ "$SKIP_BUILD" -eq 0 ]; then
  echo ""
  echo "==> Building WASM contracts..."
  cargo build --workspace --target wasm32v1-none --release --locked
  echo "   Build complete."
else
  echo "==> Skipping WASM build (--skip-build)."
fi

# ── 3. Register local network with stellar-cli ─────────────────────────────────
echo ""
echo "==> Registering local network with stellar-cli..."
stellar network add "$NETWORK" \
  --rpc-url "$RPC_URL" \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  2>/dev/null || true   # ignore "already exists"

# ── 4. Create / fund deployer keypair ──────────────────────────────────────────
echo ""
echo "==> Setting up deployer identity..."
stellar keys generate "$KEY_NAME" --network "$NETWORK" 2>/dev/null || true
ADMIN_ADDRESS="$(stellar keys address "$KEY_NAME")"
echo "   Deployer: $ADMIN_ADDRESS"

echo "==> Funding via friendbot..."
for i in $(seq 1 30); do
  if stellar keys fund "$KEY_NAME" --network "$NETWORK" 2>/dev/null; then
    break
  fi
  echo "   Friendbot not ready, retrying (${i}/30)..."
  sleep 2
done

DEPLOYER_SECRET="$(stellar keys show "$KEY_NAME" 2>/dev/null || \
  stellar keys secret "$KEY_NAME" 2>/dev/null || \
  cat ~/.config/stellar/identity/"$KEY_NAME".toml 2>/dev/null | grep secret | awk '{print $3}' || \
  true)"

# ── 5. Deploy + initialise + wire ─────────────────────────────────────────────
if [ "$SKIP_DEPLOY" -eq 0 ]; then
  echo ""
  echo "==> Deploying all contracts to local sandbox..."

  XLM_TOKEN="$(stellar contract asset deploy \
    --asset native \
    --source "$KEY_NAME" \
    --network "$NETWORK" \
    2>/dev/null \
    || stellar contract id asset --asset native --network "$NETWORK")"

  read -r _ _ FEE_CONFIG_JSON <<<"$(python3 scripts/lib/read_network_config.py config/testnet.json "$XLM_TOKEN")"

  _deploy_one() {
    local pkg_name="$1"; shift
    local wasm_src="$WASM_DIR/scoutchain_${pkg_name}.wasm"
    local wasm_opt="$WASM_DIR/scoutchain_${pkg_name}.optimized.wasm"
    # Try the modern build+optimize path first; fall back to manual optimize or raw copy.
    stellar contract build \
      --package "scoutchain-${pkg_name//_/-}" \
      --optimize \
      --target-dir target \
      >/dev/null 2>&1 \
      || stellar contract optimize \
           --wasm "$wasm_src" \
           --wasm-out "$wasm_opt" \
           >/dev/null 2>&1 \
      || cp "$wasm_src" "$wasm_opt"
    stellar contract deploy \
      --wasm "$wasm_opt" \
      --source "$KEY_NAME" \
      --network "$NETWORK" \
      -- "$@" | grep -oE 'C[A-Z0-9]{55}' | tail -1
  }

  ADMIN="$ADMIN_ADDRESS"
  REG_ID="$(_deploy_one registration \
    --admin "$ADMIN" \
    --xlm-token "$XLM_TOKEN" \
    --fee-config "$FEE_CONFIG_JSON")"
  echo "   registration:  $REG_ID"

  VER_ID="$(_deploy_one verification \
    --admin "$ADMIN" \
    --registration-contract "$REG_ID")"
  echo "   verification:  $VER_ID"

  PROG_ID="$(_deploy_one progress \
    --admin "$ADMIN" \
    --registration-contract "$REG_ID")"
  echo "   progress:      $PROG_ID"

  SCOUT_ID="$(_deploy_one scout_access \
    --admin "$ADMIN" \
    --xlm-token "$XLM_TOKEN" \
    --fee-config "$FEE_CONFIG_JSON" \
    --registration-contract "$REG_ID")"
  echo "   scout_access:  $SCOUT_ID"

  # Wire cross-contract pointers
  echo ""
  echo "==> Wiring contracts..."
  stellar contract invoke --id "$VER_ID" --source "$KEY_NAME" --network "$NETWORK" \
    -- set_progress_contract --progress-contract "$PROG_ID" >/dev/null
  stellar contract invoke --id "$SCOUT_ID" --source "$KEY_NAME" --network "$NETWORK" \
    -- set_progress_contract --progress-contract "$PROG_ID" >/dev/null
  stellar contract invoke --id "$SCOUT_ID" --source "$KEY_NAME" --network "$NETWORK" \
    -- set_verification_contract --verification-contract "$VER_ID" >/dev/null 2>&1 || true

  # Write .env.contracts
  cat > "$REPO_ROOT/.env.contracts" << ENVEOF
REGISTRATION_CONTRACT_ID=$REG_ID
VERIFICATION_CONTRACT_ID=$VER_ID
PROGRESS_CONTRACT_ID=$PROG_ID
SCOUT_ACCESS_CONTRACT_ID=$SCOUT_ID
NETWORK=$NETWORK
NETWORK_PASSPHRASE=$NETWORK_PASSPHRASE
XLM_TOKEN_ADDRESS=$XLM_TOKEN
ADMIN_ADDRESS=$ADMIN
ENVEOF
  echo "   Written .env.contracts"
else
  echo "==> Skipping deploy (--skip-deploy). Loading existing .env.contracts..."
  # shellcheck source=/dev/null
  source "$REPO_ROOT/.env.contracts"
  ADMIN_ADDRESS="${ADMIN_ADDRESS:-$ADMIN}"
fi

# shellcheck source=/dev/null
source "$REPO_ROOT/.env.contracts"

# ── 6. Apply SQL migrations ────────────────────────────────────────────────────
echo ""
echo "==> Applying SQL migrations to PostgreSQL..."
for sql in "$REPO_ROOT"/migrations/*.sql; do
  echo "   $(basename "$sql")..."
  docker compose exec -T postgres psql -U scoutchain -d scoutchain_dev < "$sql" >/dev/null 2>&1 || true
done

# ── 7. Write .env.local for the backend ───────────────────────────────────────
echo ""
echo "==> Writing .env.local for backend..."

SEP10_SECRET="$(stellar keys show "$KEY_NAME" 2>/dev/null \
  || stellar keys secret "$KEY_NAME" 2>/dev/null \
  || echo "")"

# Generate a random 32-byte JWT secret for local dev
JWT_SECRET="$(python3 -c "import secrets; print(secrets.token_hex(32))")"

cat > "$REPO_ROOT/.env.local" << ENVEOF
DATABASE_URL=$DB_URL
JWT_SECRET=$JWT_SECRET
JWT_ALLOW_HS256_MIGRATION=true
SEP10_SIGNING_SECRET=${SEP10_SECRET:-$DEPLOYER_SECRET}
HOME_DOMAIN=localhost:3000
WEB_AUTH_DOMAIN=localhost:3000
STELLAR_NETWORK=local
NETWORK_PASSPHRASE=$NETWORK_PASSPHRASE
REGISTRATION_CONTRACT_ID=${REGISTRATION_CONTRACT_ID:-}
VERIFICATION_CONTRACT_ID=${VERIFICATION_CONTRACT_ID:-}
PROGRESS_CONTRACT_ID=${PROGRESS_CONTRACT_ID:-}
SCOUT_ACCESS_CONTRACT_ID=${SCOUT_ACCESS_CONTRACT_ID:-}
ADMIN_ADDRESS=${ADMIN_ADDRESS:-}
ENVEOF
echo "   Written .env.local"

# ── 8. Start backend ───────────────────────────────────────────────────────────
echo ""
echo "==> Building and starting backend container..."
docker compose up -d --build backend

echo "==> Waiting for backend to be healthy..."
for i in $(seq 1 24); do
  if curl -sf http://localhost:3000/health >/dev/null 2>&1; then
    echo "   Backend ready after ${i}×5 s"
    break
  fi
  sleep 5
done

# ── 9. Generate TypeScript bindings ───────────────────────────────────────────
echo ""
if command -v stellar >/dev/null 2>&1 && command -v node >/dev/null 2>&1; then
  echo "==> Generating TypeScript bindings..."
  STELLAR_NETWORK=local \
    NETWORK_PASSPHRASE="$NETWORK_PASSPHRASE" \
    bash "$REPO_ROOT/scripts/generate-bindings.sh" --network local \
    >/dev/null 2>&1 || echo "   (bindings generation skipped — run manually)"
else
  echo "==> Skipping bindings generation (stellar-cli or node not on PATH)."
fi

echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║   Stack is up!                                       ║"
echo "╠══════════════════════════════════════════════════════╣"
echo "║  Soroban RPC:  http://localhost:8000/soroban/rpc     ║"
echo "║  Backend:      http://localhost:3000                 ║"
echo "║  Postgres:     localhost:5432 (scoutchain/scoutchain)║"
echo "║                                                      ║"
echo "║  Run e2e tests:  npx jest --config e2e/jest.config.ts║"
echo "║  Tear down:      scripts/dev-down.sh                 ║"
echo "╚══════════════════════════════════════════════════════╝"
