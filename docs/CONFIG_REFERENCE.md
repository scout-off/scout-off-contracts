# ScoutChain — Configuration Reference

All environment variables consumed anywhere in this repository are listed below,
grouped by component. Copy `.env.example` to `.env` and fill in every **Required**
value before running scripts or starting the backend.

---

## Stellar / Soroban — Deployment & Contract Invocation

These variables are read by the deployment and initialization shell scripts:
`scripts/deploy.sh`, `scripts/initialize.sh`, `scripts/generate-bindings.sh`,
`scripts/setup-testnet.sh`, and `testnet/seed.sh`.

| Variable | Consumer | Required / Optional | Notes |
|---|---|---|---|
| `DEPLOYER_SECRET` | `scripts/deploy.sh`, `scripts/initialize.sh`, `testnet/seed.sh` | **Required** | Stellar secret key (`S...`) used to sign deployment and invocation transactions |
| `ADMIN_ADDRESS` | `scripts/initialize.sh`, `testnet/seed.sh` | **Required** | Stellar G-address set as admin on all four contracts. Cannot be changed after `initialize`. |
| `XLM_TOKEN_ADDRESS` | `scripts/initialize.sh` | **Required** | Native XLM token contract address. Testnet: `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`. Mainnet: `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` |
| `STELLAR_NETWORK` | `scripts/deploy.sh`, `scripts/initialize.sh`, `scripts/generate-bindings.sh` | Optional | `testnet` or `mainnet`. Defaults to `testnet`. |
| `HORIZON_URL` | `testnet/seed.sh` | Optional | Horizon REST API endpoint. Default: `https://horizon-testnet.stellar.org` |
| `SOROBAN_RPC_URL` | `scripts/deploy.sh`, `scripts/initialize.sh` | Optional | Soroban JSON-RPC endpoint. Default: `https://soroban-testnet.stellar.org` |

---

## Contract IDs

Written to `.env.contracts` by `scripts/deploy.sh`. Must be copied into the backend
and frontend repos after deployment. Also consumed directly by
`scripts/initialize.sh`, `scripts/generate-bindings.sh`, `testnet/seed.sh`, and
`scripts/reconcile-indexer.js`.

| Variable | Consumer | Required / Optional | Notes |
|---|---|---|---|
| `REGISTRATION_CONTRACT_ID` | `scripts/initialize.sh`, `scripts/generate-bindings.sh`, `testnet/seed.sh`, `scripts/reconcile-indexer.js` | **Required** (post-deploy) | Contract ID of the deployed `registration` contract |
| `VERIFICATION_CONTRACT_ID` | `scripts/initialize.sh`, `scripts/generate-bindings.sh`, `testnet/seed.sh` | **Required** (post-deploy) | Contract ID of the deployed `verification` contract |
| `PROGRESS_CONTRACT_ID` | `scripts/initialize.sh`, `scripts/generate-bindings.sh`, `testnet/seed.sh` | **Required** (post-deploy) | Contract ID of the deployed `progress` contract |
| `SCOUT_ACCESS_CONTRACT_ID` | `scripts/initialize.sh`, `scripts/generate-bindings.sh`, `testnet/seed.sh`, `scripts/reconcile-indexer.js` | **Required** (post-deploy) | Contract ID of the deployed `scout_access` contract |

---

## WASM Hashes

Written to `.env.contracts` by `scripts/deploy.sh`. Used to verify the on-chain WASM
binary identity matches the locally built artifact.

| Variable | Consumer | Required / Optional | Notes |
|---|---|---|---|
| `REGISTRATION_CONTRACT_WASM_HASH` | `scripts/deploy.sh` (output) | Optional | SHA-256 hash of the deployed registration WASM |
| `VERIFICATION_CONTRACT_WASM_HASH` | `scripts/deploy.sh` (output) | Optional | SHA-256 hash of the deployed verification WASM |
| `PROGRESS_CONTRACT_WASM_HASH` | `scripts/deploy.sh` (output) | Optional | SHA-256 hash of the deployed progress WASM |
| `SCOUT_ACCESS_CONTRACT_WASM_HASH` | `scripts/deploy.sh` (output) | Optional | SHA-256 hash of the deployed scout_access WASM |

---

## Backend / Node.js

Read by the TypeScript application server and the Node.js reconciler script.

| Variable | Consumer | Required / Optional | Notes |
|---|---|---|---|
| `DATABASE_URL` | `prisma/schema.prisma`, `scripts/reconcile-indexer.js` | **Required** | PostgreSQL connection string used by Prisma ORM and the indexer reconciler. Format: `postgresql://USER:PASSWORD@HOST:PORT/DATABASE` |
| `JWT_SECRET` | `src/middleware/auth.ts` | **Required** | Secret used to sign and verify JWT bearer tokens. Use a cryptographically random value (`openssl rand -hex 32`). Never commit the real value. |

---

## Notes

- Never commit a populated `.env` file. It is listed in `.gitignore`.
- After running `scripts/deploy.sh`, copy `.env.contracts` values into your backend
  and frontend `.env` files — they are not automatically forwarded.
- For mainnet deployments, verify `config/mainnet.json` has no placeholder values
  (`FILL_IN_BEFORE_USE`) before running `scripts/deploy.sh mainnet`.
# Configuration Reference

This document describes every field in `config/testnet.json` and `config/mainnet.json`. These files are the **single source of truth** for network-specific values used by deployment scripts, initialization, CI pipelines, and the backend indexer.

---

## Shared Fields (both files)

| Field | Type | Description | Source / Notes |
|-------|------|-------------|----------------|
| `network` | `"testnet" \| "mainnet"` | Network identifier. Must match the filename (`testnet.json` → `"testnet"`). | Used by scripts to select the correct config file. |
| `rpc_url` | string | Soroban RPC endpoint URL. | **Testnet:** `https://soroban-testnet.stellar.org`<br>**Mainnet:** Use a paid provider (e.g., ValidationCloud, Blockdaemon) — placeholder `FILL_IN_BEFORE_USE` must be replaced. |
| `horizon_url` | string | Horizon API endpoint for transaction submission / event streaming. | **Testnet:** `https://horizon-testnet.stellar.org`<br>**Mainnet:** `https://horizon.stellar.org` |
| `network_passphrase` | string | Stellar network passphrase for transaction signing. | **Testnet:** `Test SDF Network ; September 2015`<br>**Mainnet:** `Public Global Stellar Network ; September 2015` |
| `friendbot_url` | string \| null | Friendbot faucet URL for funding test accounts. | **Testnet:** `https://friendbot.stellar.org`<br>**Mainnet:** `null` (no faucet on mainnet) |
| `xlm_token_address` | string | Contract address of the native XLM token (SAC-0001). | **Sourced from Stellar's official SAC registry.**<br>**Testnet:** `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`<br>**Mainnet:** `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA`<br>**Owner:** Release Engineer verifies before each deployment. |
| `admin_secret` | string | Secret key for the platform admin account. Used by RUNBOOK scripts for `require_auth()` on privileged operations (pause, unpause, rotation, health checks). | **Must be set in `.env`** — not stored in config files. Sensitivity: **high**. |

---

## Field Details

### `rpc_url`
- **Purpose:** All `stellar contract` CLI calls (deploy, invoke, install, upgrade) hit this endpoint.
- **Testnet:** Public SDF endpoint, rate-limited but free.
- **Mainnet:** **Must use a reliable paid provider.** The placeholder in `mainnet.json` (`FILL_IN_BEFORE_USE`) will cause deployment to fail if not replaced.
- **CI usage:** GitHub Actions workflows inject this via secrets.

### `horizon_url`
- **Purpose:** Backend indexer and scripts use Horizon for:
  - Streaming contract events (`/events` endpoint)
  - Fetching transaction details for reconciliation
  - Health checks
- **Failover:** If primary Horizon is down, update this field and redeploy indexer.

### `network_passphrase`
- **Purpose:** Used by the Stellar SDK to sign transactions for the correct network.
- **Immutable per network.** Do not change.

### `friendbot_url`
- **Purpose:** `scripts/setup-testnet.sh` and local dev scripts call this to fund test accounts.
- **Mainnet is `null`** — attempting to use friendbot on mainnet will error.

### `xlm_token_address`
- **Purpose:** The `scout_access` contract stores this in instance storage during `initialize` and uses it for:
  - Fee payments (contact fees, subscription fees)
  - Withdrawals to admin
- **Source of truth:** [Stellar Asset Contract (SAC) registry](https://github.com/stellar/stellar-asset-contract-registry)
- **Verification step (Release Engineer):**
  1. Check SAC registry for latest testnet/mainnet XLM contract address
  2. Update both `config/testnet.json` and `config/mainnet.json`
  3. Update `.env.example` `XLM_TOKEN_ADDRESS`
  4. Commit before deployment

---

## Where These Values Are Used

| Script / Component | Fields Consumed |
|--------------------|-----------------|
| `scripts/deploy.sh` | `rpc_url`, `network_passphrase` |
| `scripts/initialize.sh` | `rpc_url`, `network_passphrase`, `xlm_token_address` (via `.env`) |
| `scripts/upgrade.sh` | `rpc_url`, `network_passphrase` |
| `scripts/migrate-contract.sh` | All fields |
| `scripts/health-check.sh` | `rpc_url`, `horizon_url` |
| `scripts/generate-bindings.sh` | `rpc_url`, `network_passphrase` |
| `scripts/verify-cross-contract-wiring.sh` | `rpc_url`, `network_passphrase` |
| Backend indexer (TypeScript) | `horizon_url`, `network_passphrase`, `xlm_token_address` |
| CI workflows (`.github/workflows/*.yml`) | `rpc_url`, `horizon_url`, `network_passphrase` (via secrets) |

---

## Mainnet Deployment Checklist (Config)

Before running `./scripts/deploy.sh mainnet`:

- [ ] `config/mainnet.json` `rpc_url` → real paid provider URL (no `FILL_IN_BEFORE_USE`)
- [ ] `config/mainnet.json` `xlm_token_address` → verified against latest SAC registry
- [ ] `.env` `DEPLOYER_SECRET` → mainnet deployer secret key
- [ ] `.env` `ADMIN_ADDRESS` → mainnet admin G-address
- [ ] `.env` `XLM_TOKEN_ADDRESS` → matches `config/mainnet.json`
- [ ] CI secrets updated: `RPC_URL`, `HORIZON_URL`, `NETWORK_PASSPHRASE`

---

## Contract-Level Settings

### Registration Cooldown (`set_reg_cooldown` / `get_reg_cooldown`)

| Setting | Default | Bounds | Description |
|---------|---------|--------|-------------|
| `reg_cooldown_secs` | `86400` (24h) | `0..=604800` (0 = disabled, max 7 days) | Per-wallet cooldown between `register_player` and `register_scout` calls. Set via `set_reg_cooldown` (admin only). Emits `reg_cooldown_updated` event. |

### Verification Cooldown (`set_reg_cooldown` / `get_reg_cooldown`)

| Setting | Default | Bounds | Description |
|---------|---------|--------|-------------|
| `reg_cooldown_secs` | `0` (disabled) | `0..=604800` (0 = disabled, max 7 days) | Per-wallet cooldown between `register_validator` calls. Set via `set_reg_cooldown` (admin only). |

---

## Updating Config Values

1. Edit `config/testnet.json` or `config/mainnet.json`
2. Run `./scripts/health-check.sh <network>` to verify RPC/Horizon connectivity
3. Commit changes — CI will pick up new values on next deploy

> **Never** hardcode these values in scripts. Always read from the config files or `.env.contracts` (which `deploy.sh` generates from these configs).
