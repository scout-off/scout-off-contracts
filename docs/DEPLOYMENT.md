# Deployment Guide

> **See also:** [Glossary](GLOSSARY.md)

## Prerequisites

- Rust + `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- Stellar CLI: https://developers.stellar.org/docs/tools/developer-tools/cli/install-stellar-cli
- A funded Stellar keypair for deployment

## Contract Deployment Order

The four contracts must be deployed in the following order. Deploying out of
sequence will cause `initialize.sh` cross-contract wiring to fail with a
missing contract ID error.

1. **`registration`** — Deployed first because it owns player and scout
   identity records. All other contracts reference `player_id` values that
   originate here. No dependency on any other contract.

2. **`verification`** — Deployed second because `approve_milestone` must
   cross‑call `progress.advance_level`. The progress contract address is wired
   in by `initialize.sh` *after* both verification and registration are deployed.

   **Deployment order guidance:**

   - ✅ *Safe*: Deploy `verification` **before** `registration`.
   - ❌ *Breaks milestone flow*: Deploy `verification` **after** `progress` **and** skip deploying `registration`.

3. **`progress`** — Deployed third. Holds the four-tier level state machine.
   Receives calls only from the verification contract (production) or directly
   (test). Must exist before `initialize.sh` runs `set_progress_contract` on
   the verification contract.

4. **`scout_access`** — Deployed last because it depends on the progress
   contract address for `log_trial_offer → advance_level` cross-calls. It also
   references player IDs from registration at runtime.

> **Warning — do not deploy `progress` before `registration`.**
> `initialize.sh` calls `set_progress_contract` on the registration contract
> after deploying progress. If registration has not been deployed yet, the
> script will fail and leave the system in a partially initialized state
> requiring manual cleanup.

> **Warning — do not run `initialize.sh` before all four contracts are
> deployed.** The script reads all four contract IDs from `.env.contracts`. A
> missing ID causes the wiring steps to silently pass the wrong address,
> breaking cross-contract calls at runtime.

`deploy.sh` respects this order automatically. If you are deploying manually,
follow the numbered sequence above and write each contract ID to `.env.contracts`
before proceeding to the next contract.

---

## Step-by-step

### 1. Configure environment

```bash
cp .env.example .env
# Fill in DEPLOYER_SECRET, ADMIN_ADDRESS, XLM_TOKEN_ADDRESS
```

### 2. Deploy all contracts

```bash
chmod +x scripts/deploy.sh
./scripts/deploy.sh testnet
# Contract IDs written to .env.contracts
```

### 3. Initialize and wire contracts

```bash
chmod +x scripts/initialize.sh
./scripts/initialize.sh testnet
# Sets admin, fee config, and wires all cross-contract links:
# - Verification → Progress: verification.set_progress_contract
# - Registration ← Progress: registration.set_progress_contract
# - Progress → Verification: progress.set_verification_contract
# - Progress → Registration: progress.set_registration_contract
# - Scout Access → Progress: scout_access.set_progress_contract
```

### 4. Generate TypeScript bindings

```bash
chmod +x scripts/generate-bindings.sh
./scripts/generate-bindings.sh testnet
# Bindings written to bindings/{contract}/
```

### 5. Seed testnet with demo data (optional)

```bash
chmod +x testnet/seed.sh
./testnet/seed.sh
```

### 6. Run the database migration

Copy `migrations/001_initial_schema.sql` to your backend repo and run it against PostgreSQL:

```bash
psql $DATABASE_URL -f migrations/001_initial_schema.sql
```

## Mainnet checklist

- [ ] Audit all four contracts
- [ ] Replace testnet XLM token address with mainnet address in `.env`
- [ ] Set `STELLAR_NETWORK=mainnet` and update RPC/Horizon URLs
- [ ] Run `./scripts/deploy.sh mainnet`
- [ ] Run `./scripts/initialize.sh mainnet`
- [ ] Verify all contract IDs in `.env.contracts`
- [ ] Regenerate bindings: `./scripts/generate-bindings.sh mainnet`

## Upgrading a Deployed Contract

All four contracts expose an `upgrade(new_wasm_hash)` function (admin auth required). The admin address is stored in **persistent** storage so it survives the WASM swap. Upgrading replaces only the executable WASM — **the contract ID stays the same**, so all existing clients, integrations, and indexed data continue to work without any address change.

Instance storage (Initialized, Paused, counters, fee config, contract links) is **not** automatically wiped during an upgrade, but values must be re-verified after each WASM swap in case the new code changes the storage layout or if instance TTL has drifted close to expiry.

### Scripted upgrade (recommended)

`scripts/upgrade.sh` automates the five-step procedure below, including the keypair guard, instance-state snapshot, WASM installation, the `upgrade()` call, health check, and a per-contract post-upgrade checklist.

```bash
# Build first
cargo build --target wasm32v1-none --release

# Then upgrade a single contract
./scripts/upgrade.sh testnet scout_access \
  target/wasm32v1-none/release/scoutchain_scout_access.wasm

# Other contract names: registration | verification | progress
```

The script prints a post-upgrade checklist specific to the contract being upgraded (re-wiring links, restoring fee config, regenerating bindings).

### Manual upgrade procedure

**Step 1 — Snapshot current on-chain state** (before upgrading)

```bash
# scout_access: save fee config
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  --network testnet -- get_fee_config

# All contracts: note current version
stellar contract invoke --id $CONTRACT_ID --network testnet -- version
```

**Step 2 — Build and install the new WASM**

```bash
cargo build --target wasm32v1-none --release

stellar contract install \
  --source $DEPLOYER_SECRET \
  --network testnet \
  --wasm target/wasm32v1-none/release/<contract_name>.wasm
# Prints the new wasm hash → NEW_WASM_HASH
```

**Step 3 — Call `upgrade`** (must be signed by the admin address)

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source $DEPLOYER_SECRET \
  --network testnet \
  -- upgrade \
  --new_wasm_hash <NEW_WASM_HASH>
```

**Step 4 — Verify the contract is healthy**

```bash
stellar contract invoke --id $CONTRACT_ID --network testnet -- health
stellar contract invoke --id $CONTRACT_ID --network testnet -- version
```

**Step 5 — Re-apply instance state** (if needed)

For `scout_access`, restore fee config and progress contract link:

```bash
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- update_fee_config --fee_config '<saved JSON>'

stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_progress_contract --addr $PROGRESS_CONTRACT_ID
```

For `verification`, re-wire the progress contract link:

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_progress_contract \
  --progress_contract $PROGRESS_CONTRACT_ID
```

For `progress`, re-wire both cross-contract links:

```bash
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_verification_contract --addr $VERIFICATION_CONTRACT_ID

stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- set_registration_contract --addr $REGISTRATION_CONTRACT_ID
```

**Step 6 — Regenerate TypeScript bindings** (if the ABI changed)

```bash
./scripts/generate-bindings.sh testnet
```

### Address migration (new contract ID)

If a bug cannot be fixed via `upgrade()` (e.g. the storage layout must change in a way that requires a fresh deploy), you must migrate to a new contract address. This is a breaking change — all clients and the off-chain indexer must be updated.

Migration procedure:

1. Deploy the new contract: `./scripts/deploy.sh testnet` (or deploy just the affected contract manually).
2. Initialize the new contract: `./scripts/initialize.sh testnet`.
3. Pause the old contract so no new state is written: `stellar contract invoke --id $OLD_ID -- pause_contract`.
4. Replay any off-chain events against the new contract to seed initial state (use the backend indexer's event log).
5. Update `.env.contracts` with the new contract ID.
6. Regenerate TypeScript bindings: `./scripts/generate-bindings.sh testnet`.
7. Deploy the updated backend and frontend with the new contract ID.
8. Announce the migration in release notes with the old and new contract IDs.

### What survives an upgrade

| Data | Storage | Survives upgrade? |
|------|---------|-------------------|
| Admin address | Persistent | ✅ Yes |
| Player / scout profiles | Persistent | ✅ Yes |
| Validator registry | Persistent | ✅ Yes |
| Milestone / subscription records | Persistent | ✅ Yes |
| Contact records and scout indexes | Persistent | ✅ Yes |
| Initialized flag | Instance | ⚠️ Must re-verify |
| Paused flag | Instance | ⚠️ Must re-verify |
| Fee config (scout_access) | Instance | ⚠️ Must re-verify / re-set |
| XLM token address (scout_access) | Instance | ⚠️ Must re-verify |
| Progress contract link (all) | Instance | ⚠️ Must re-wire |

> **Note:** On Stellar, instance storage is **not** automatically wiped during an `upgrade()` call — only the contract code (WASM) is replaced. The table above reflects the risk if the new WASM changes the storage layout or if instance TTL expires before the upgrade completes. Always re-verify instance state after an upgrade using `scripts/upgrade.sh` or the manual steps above.

## Common Mistakes

**Milestones approved but player levels don't advance**
You skipped the cross-contract wiring step. `approve_milestone` calls `advance_level` on the progress contract, but only if all links have been set. Fix it by running:

```bash
./scripts/initialize.sh testnet
```

Or manually:

```bash
# 1. Verification → Progress link
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- set_progress_contract \
  --progress_contract $PROGRESS_CONTRACT_ID

# 2. Registration ← Progress link
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  -- set_progress_contract \
  --addr $PROGRESS_CONTRACT_ID

# 3. Progress → Verification link
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_verification_contract \
  --addr $VERIFICATION_CONTRACT_ID

# 4. Progress → Registration link
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_registration_contract \
  --addr $REGISTRATION_CONTRACT_ID

# 5. Scout Access → Progress link
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_progress_contract \
  --addr $PROGRESS_CONTRACT_ID
```

This must be done once after every fresh deployment.

---

## Rollback Procedure

If a deployment partially fails (e.g. `registration` and `verification` succeed but `progress`
fails), the system ends up in an inconsistent state. The rollback procedure restores the last
known good contract addresses automatically.

### How it works

`deploy.sh` writes a snapshot of the current `.env.contracts` to `.env.contracts.snapshot`
**before** making any changes. If a deployment fails, you can restore from that snapshot.

### Automatic rollback (CI)

If the CI deploy pipeline fails, it prints rollback instructions. Run:

```bash
./scripts/rollback.sh testnet   # or mainnet
```

This script:
1. Restores `.env.contracts` from `.env.contracts.snapshot`
2. Runs `scripts/health-check.sh` to verify the restored contracts are responsive

### Manual rollback

```bash
# Inspect the snapshot
cat .env.contracts.snapshot

# Restore it
cp .env.contracts.snapshot .env.contracts

# Verify contracts are healthy
./scripts/health-check.sh testnet
```

### When there is no snapshot

A snapshot is only created when `.env.contracts` already exists at the start of a deployment
(i.e. there was a previous successful deployment). For a first-time deployment failure there is
no snapshot — you must re-deploy from scratch:

```bash
./scripts/deploy.sh testnet
./scripts/initialize.sh testnet
```
