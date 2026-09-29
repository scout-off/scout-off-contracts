# Deployment Guide

## Prerequisites

<!-- Note: XLM token address source of truth -->
> **Note:** The `xlm_token_address` values in `config/mainnet.json` and `config/testnet.json` (and the corresponding entry in `.env.example`) are sourced from Stellar's official SAC registry. The team member responsible for verifying and updating these addresses before each deployment is the **Release Engineer**. Ensure the addresses match the latest SAC documentation before deploying.
>
> **Full field reference:** See [`docs/CONFIG_REFERENCE.md`](CONFIG_REFERENCE.md) for a complete description of every field in `config/testnet.json` and `config/mainnet.json`, including where each value is used and mainnet deployment requirements.

- Rust + `wasm32v1-none` target: `rustup target add wasm32v1-none`
  (Note: `rust-toolchain.toml` in this repo auto-selects the correct target when you run `cargo` from the project directory, so this step is only needed for manual `stellar contract install` invocations outside of `cargo`.)
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

### Re-wiring the progress contract link (verification)

`verification.set_progress_contract` is a **first-time-only** setter: it
returns `AlreadyConfigured` on every call after the first, so the wrong
address is never silently overwritten. If you need to point `verification`
at a different progress contract — a redeploy, a bad address on the first
run, or an `initialize.sh` re-run — call **`update_progress_contract`**
instead:

```bash
stellar contract invoke \
  --id $VERIFICATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- update_progress_contract \
  --progress_contract $NEW_PROGRESS_CONTRACT_ID
```

Both functions emit `progress_contract_updated` with the new address, so
off-chain indexers see the change either way. As of issue #1041 both also
emit `wiring_updated` (address + a bumped re-wiring epoch) alongside it — see
[`docs/WIRING_REGISTRY_DESIGN.md`](WIRING_REGISTRY_DESIGN.md).
`verification.set_registration_contract` carries the identical first-call-only
guard (with `update_registration_contract` as its own escape hatch) — it
predates this doc section but works the same way.

Every other `set_*_contract` setter across all four contracts — including
`registration.set_progress_contract` and `scout_access.set_progress_contract`
/ `set_registration_contract` — has no such guard: they can always be
re-invoked to re-wire the link, and `scout_access` also exposes
`update_progress_contract` as an alias for the same call so the same verb
works across contracts. This asymmetry (verification's two links
first-call-only, every other link freely re-settable) is deliberate and
preserved for backward compatibility with already-deployed contracts — see
`docs/WIRING_REGISTRY_DESIGN.md`'s re-wiring policy section for why it was
not homogenised further.

`./scripts/initialize.sh` is idempotent with respect to both of
verification's guarded links: if `set_progress_contract` or
`set_registration_contract` on `verification` fails with `AlreadyConfigured`
(e.g. because the script is being re-run), it automatically falls back to
`update_progress_contract` / `update_registration_contract` instead of
aborting.

### Checking wiring is actually consistent, not just "set"

Every contract now exposes `get_wiring_state()`, returning each peer
pointer's address **and** a monotonically-incrementing `epoch` bumped on
every re-wiring call (`scoutchain_shared_types::WiringLink`). Because Soroban
has no atomic multi-contract transaction, a re-wiring operation touching
several contracts (e.g. redeploying `progress` requires updating three
separate `ProgressContract` pointers on `verification`, `registration`, and
`scout_access`) can fail or be interrupted partway through, leaving some
pointers updated and others stale. `scripts/verify-cross-contract-wiring.sh`
detects exactly this: it groups the platform's eight peer-address pointers by
which contract they target, and flags a group as `PARTIAL` when some members
agree with the target's actual deployed address and others don't — distinct
from `NEVER_CONFIGURED` (nobody has wired it yet) and `FULLY_WIRED`. Run it
with `--repair` to print just the corrective `stellar contract invoke`
commands for whatever is inconsistent.

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
# Sets admin, fee config, and wires all eight cross-contract links:
# - Verification → Progress: verification.set_progress_contract
# - Registration ← Progress: registration.set_progress_contract
# - Verification → Registration: verification.set_registration_contract
# - Progress → Verification: progress.set_verification_contract
# - Progress → Registration: progress.set_registration_contract
# - Progress → Scout Access: progress.set_scout_access_contract
# - Scout Access → Progress: scout_access.set_progress_contract
# - Scout Access → Registration: scout_access.set_registration_contract
#
# Then gates success on scripts/verify-cross-contract-wiring.sh — the script
# aborts with a non-zero exit if any link is missing, misconfigured, or
# partially re-wired, rather than declaring success just because none of the
# individual `stellar contract invoke` calls above returned an error.
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

### 6. Verify deployment health and wiring (recommended)

After deploying and initializing, run the combined readiness check to confirm
all four contracts are healthy (initialized and not paused) and all eight
cross-contract wiring links are correctly and consistently set — not just
present, but agreeing with each other, catching a partially-applied re-wiring
(see [Checking wiring is actually consistent](#checking-wiring-is-actually-consistent-not-just-set)
above) — before routing any traffic:

```bash
chmod +x scripts/full-readiness-check.sh
./scripts/full-readiness-check.sh testnet
```

This prints a combined summary table with ✅/❌/⚠️ status for every health and
wiring check in a single command.  If any check fails, the script exits
non-zero and names the failing check explicitly.

The two underlying scripts remain available for targeted debugging:

- `scripts/health-check.sh testnet` — init/pause status only
- `scripts/verify-cross-contract-wiring.sh testnet` — wiring links only

### 7. Run the database migration

Copy the migration files to your backend repo and run them against PostgreSQL in
numeric order. **Run every file** — skipping any migration leaves tables, columns,
or indexes missing and will cause silent indexer errors at runtime.

```bash
psql $DATABASE_URL -f migrations/001_initial_schema.sql
psql $DATABASE_URL -f migrations/002_cursor_upsert_helper.sql
psql $DATABASE_URL -f migrations/003_diagnostic_events.sql
psql $DATABASE_URL -f migrations/004_scout_subscriptions_auto_renew.sql
psql $DATABASE_URL -f migrations/005_evidence_access_grants.sql
psql $DATABASE_URL -f migrations/006_dispute_jury.sql
psql $DATABASE_URL -f migrations/007_milestone_flags.sql
```

All migration files are idempotent (`CREATE TABLE IF NOT EXISTS`,
`ALTER TABLE … ADD COLUMN IF NOT EXISTS`): re-running against an already-migrated
database is safe.

`001_initial_schema.sql` creates all fourteen base tables and seeds the
`indexer_cursor` row so the indexer can `SELECT` it on first startup.

`002_cursor_upsert_helper.sql` adds the `advance_indexer_cursor(p_ledger BIGINT)`
helper function. Call it from the indexer after processing each batch of Horizon
events instead of hand-writing the `ON CONFLICT DO UPDATE` clause each time:

```sql
SELECT advance_indexer_cursor(42391);
```

The function updates `last_ledger` only when the supplied value is greater than the
stored value, so replaying an old batch never accidentally rewinds the cursor.

`003_diagnostic_events.sql` adds the `diagnostic_events` table for off-chain
diagnostic event logging.

`004_scout_subscriptions_auto_renew.sql` adds the `auto_renew` column to
`scout_subscriptions` so the indexer can track per-scout auto-renewal opt-in.

`005_evidence_access_grants.sql` adds the `evidence_access_grants` table —
the off-chain mirror of `scout_access.EvidenceAccessGrant`.

`006_dispute_jury.sql` adds jury-escalation columns to `milestone_disputes` and
creates the `dispute_votes` table for per-validator audit trail.

`007_milestone_flags.sql` adds the `milestone_flags` and `revocation_records`
tables for the validator-revocation cascade re-review system.

### Resetting the Indexer Cursor

To replay all on-chain events from genesis — for example after a full database wipe,
a failed reindex, or when setting up a new environment from scratch — reset the
cursor to ledger 0 before restarting the indexer:

```sql
-- 1. Stop the indexer process first.

-- 2. Optionally truncate derived tables to avoid duplicate-key errors
--    when events are reprocessed (order matters due to foreign keys):
TRUNCATE TABLE
    milestone_disputes,
    milestones,
    trial_offers,
    contact_records,
    scout_subscriptions,
    fee_config_history,
    fee_withdrawals,
    admin_transfers,
    validator_history,
    player_level_history,
    players,
    scouts,
    validators
CASCADE;

-- 3. Reset the cursor.
UPDATE indexer_cursor
SET last_ledger = 0,
    updated_at  = NOW()
WHERE id = 1;

-- 4. Restart the indexer — it will stream events from ledger 0.
```

### 8. Seed migrated state (optional)

For fresh deployments of an existing production dataset, use the admin-only
seeding entrypoints to replay exported player/scout profiles without requiring
their wallet signatures:

```bash
stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- admin_seed_player \
  --player_id <id> \
  --wallet <G-address> \
  --vitals '{"age":25,"position":"Forward","region":"Europe","nationality":"FR"}' \
  --ipfs_hashes '["QmHash"]' \
  --registered_at <unix_ts> \
  --level <0-3>

stellar contract invoke --id $REGISTRATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- admin_seed_scout \
  --scout_id <id> \
  --wallet <G-address> \
  --region "Europe" \
  --verified false \
  --registered_at <unix_ts>
```

> **Warning:** These functions bypass wallet authentication.  They should only
> be used during a controlled migration replay before the contract serves any
> real wallet-signed registrations.

### 9. Verify indexer consistency

```bash
node scripts/reconcile-indexer.js
```

The script compares on-chain state against the local database and reports
discrepancies for `players.deactivated` and `scouts.verified`.
If you only want to re-process events from a specific ledger (partial replay),
replace `0` with the desired starting ledger sequence number.

## Mainnet checklist

- [ ] Audit all four contracts
- [ ] Review storage TTL cost model (`docs/STORAGE_COST_MODEL.md`) and budget for ongoing TTL renewal
- [ ] Replace testnet XLM token address with mainnet address in `.env`
- [ ] Set `STELLAR_NETWORK=mainnet` and update RPC/Horizon URLs
- [ ] Run `./scripts/deploy.sh mainnet`
- [ ] Run `./scripts/initialize.sh mainnet`
- [ ] Verify all contract IDs in `.env.contracts`
- [ ] **Run `./scripts/full-readiness-check.sh mainnet`** — confirms all four contracts are healthy and all eight wiring links are consistently set, with no partial re-wiring (recommended one-command post-deploy check)
- [ ] Regenerate bindings: `./scripts/generate-bindings.sh mainnet`
- [ ] Review [docs/STORAGE_COST_MODEL.md](STORAGE_COST_MODEL.md) and confirm the projected monthly storage rent is within budget at expected launch-day scale. Re-measure rent figures if the Stellar fee schedule has changed since the document's last-reviewed date.

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

For `verification`, re-wire the progress contract link. Instance storage
(including the `ProgressContractSet` guard flag) survives an `upgrade()`
call, so `set_progress_contract` will fail with `AlreadyConfigured` here —
use `update_progress_contract` instead:

```bash
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  --source $ADMIN_ADDRESS --network testnet \
  -- update_progress_contract \
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

> **See [`docs/MIGRATION_GAPS.md`](MIGRATION_GAPS.md) for the canonical per-category
> list of what can and cannot be automatically migrated, including in-flight milestone
> disputes and other gaps not covered in this section.**

If a bug cannot be fixed via `upgrade()` (e.g. the storage layout must change in a way that requires a fresh deploy), you must migrate to a new contract address. This is a breaking change — all clients and the off-chain indexer must be updated.

This is the highest-risk operation in the deployment lifecycle, so most of it is
now automated by **`scripts/migrate-contract.sh`**, an orchestrator that chains
the existing scripts in the order below with a `--dry-run` preview and an
interactive confirmation gate before every state-mutating step. Run a dry run
first:

```bash
# Preview every action without executing anything:
./scripts/migrate-contract.sh testnet --dry-run

# Then run for real (prompts y/N before each mutating step):
./scripts/migrate-contract.sh testnet
# ...or non-interactively (CI/automation): add --yes
```

`migrate-contract.sh` performs steps 1–5 below; steps 6–8 remain manual. It also
requires `DEPLOYER_SECRET` / `ADMIN_ADDRESS` / `XLM_TOKEN_ADDRESS` (same as
`deploy.sh` / `initialize.sh`).

Migration procedure:

1. **Deploy the new contract set** — `migrate-contract.sh` calls `./scripts/deploy.sh testnet` (which also snapshots the old IDs to `.env.contracts.snapshot`).
2. **Initialize + wire the new set** — via `./scripts/initialize.sh testnet`.
3. **Pause the old contracts** so no new state is written to the retired addresses — `migrate-contract.sh` invokes `pause_contract` on each old contract ID (equivalent to `stellar contract invoke --id $OLD_ID -- pause_contract`).
4. **Replay state onto the new set** — via `./scripts/replay-state.sh testnet` (also invoked automatically by the orchestrator). See the important limitation below.
5. **Health-check the new set** — via `./scripts/health-check.sh testnet`. At this point `.env.contracts` already points at the new IDs.
6. **Regenerate TypeScript bindings** (manual): `./scripts/generate-bindings.sh testnet`.
7. **Redeploy the backend and frontend** (manual) with the new contract IDs.
8. **Announce the migration** (manual) in release notes with the old and new contract IDs.

#### Step 4 in detail — automated replay scope

`scripts/replay-state.sh` reads the old contract set and replays the supported
state categories onto the new set using the admin key. It opens the migration
windows on `progress`, `verification`, and `scout_access` only for the replay
and closes them before returning, including after a failed invocation.

The automated replay covers validators, player/scout profiles, resolved player
levels, full progress history plus its Merkle root, milestones, disputes,
subscriptions, contacts, trial offers including in-flight escrow records,
auto-renew flags, and the current fee configuration plus its bounded history.
Each category is exported to timestamped JSON under `migration-export/` for
post-migration auditing and reconciliation. All seeders are idempotent and
reject conflicting records.

**Pre-authorized migration tickets** remain available for operators that do not
want to use the admin replay path: players and scouts can sign an off-chain
`MigrationAuthorization` ahead of a migration, and a relayer can redeem it via
`registration.redeem_migration_player(...)` or
`registration.redeem_migration_scout(...)`. See `docs/CONTRACT_REFERENCE.md`.

#### Testing a migration against the local sandbox

`scripts/migrate-contract-smoke-test.sh` exercises the whole path
(deploy old → seed a validator + a player → migrate → replay → **before/after
comparison**) against a local Soroban sandbox, using the same
`stellar/quickstart:testing` container as the `bindings-smoke-test` CI job. It
requires `docker` + the `stellar` CLI and is intended as a manual command (it
skips cleanly if those are unavailable):

```bash
./scripts/migrate-contract-smoke-test.sh
```

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
You skipped the cross-contract wiring step. `approve_milestone` calls `advance_level` on the progress contract, but only if all links have been set. Run the wiring diagnostic first to identify which links are missing:

```bash
./scripts/verify-cross-contract-wiring.sh testnet
```

This prints a ✅/❌ table for all eight wiring links in one command, plus a
per-target-contract consistency rollup that flags a partial re-wiring
separately from a link that was simply never configured. If any link shows
❌, fix it by running:

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

# 5. Verification → Registration link (same first-call-only guard as #1;
#    use update_registration_contract instead if this returns AlreadyConfigured)
stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
  -- set_registration_contract \
  --reg_contract $REGISTRATION_CONTRACT_ID

# 6. Progress → Scout Access link
stellar contract invoke --id $PROGRESS_CONTRACT_ID \
  -- set_scout_access_contract \
  --addr $SCOUT_ACCESS_CONTRACT_ID

# 7. Scout Access → Progress link
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_progress_contract \
  --addr $PROGRESS_CONTRACT_ID

# 8. Scout Access → Registration link
stellar contract invoke --id $SCOUT_ACCESS_CONTRACT_ID \
  -- set_registration_contract \
  --addr $REGISTRATION_CONTRACT_ID
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

> **Note on partially-initialized contracts from a failed first attempt:**
> If `deploy.sh` fails partway through (e.g. `registration` and `verification` succeed but
> `progress` fails), the contracts that *did* deploy remain on-chain with their IDs. However,
> since `.env.contracts` was never written, those contract IDs are not tracked anywhere.
> **No manual cleanup, pausing, or abandonment steps are required** — simply re-run
> `deploy.sh` and it will deploy a fresh set of contracts with new IDs. The orphaned contracts
> from the failed attempt are inert and pose no risk; they can be left as-is.
>
> **Explicitly:** there is no need to call `pause_contract` on the orphaned contracts, no need
> to manually abandon them, and no risk of them interfering with the fresh deployment. They are
> simply unused contract instances that will never be wired or called.


---

## TTL Policy (Issue #705)

### Overview

All four contracts have been redesigned to prevent silent data loss due to persistent storage archival. Player levels, validator registrations, scout subscriptions, and milestone records now use a 30-day TTL (518,400 ledgers) instead of ~3 hours (2,000 ledgers).

### What Changed

- **Progress contract:** PlayerLevel now extends TTL on every `get_level()` read.
- **Registration contract:** Player and Scout profiles extend TTL on every read.
- **Verification contract:** Milestone and Validator records extend TTL on every read; `approve_milestone` extends all related keys (Milestone, MilestoneCounter, EvidenceUsed) on write.
- **Scout Access contract:** Subscription and ContactRecord keys now use 30-day TTL.

### Deployment Impact

1. **No breaking changes to public APIs:** All function signatures remain the same.
2. **No changes to event shapes:** All event topics and data remain compatible with existing indexers.
3. **Increased TTL extension calls:** Every read of a core identity key now calls `extend_ttl`. This is visible in transaction logs but is expected and safe.

### Validation

After deployment, verify the fix:

```bash
# In any contract's test environment:
1. Register a player and advance them to Elite tier.
2. Advance the test ledger far past the old TTL threshold (~5000 ledgers).
3. Call get_level(player_id) — must return Elite, not Unverified.
```

For complete validation, see the integration tests in:
- `contracts/progress/src/lib.rs`: `test_player_level_survives_extended_dormancy_via_ttl_extension()`
- `contracts/registration/src/lib.rs`: `test_player_profile_survives_extended_dormancy_via_ttl_extension()`
- `contracts/verification/src/lib.rs`: `test_validator_and_milestone_survive_extended_dormancy_via_ttl_extension()`

### Documentation

See [`docs/TTL_POLICY.md`](TTL_POLICY.md) for:
- Detailed TTL values per contract and per DataKey
- Rationale for 30-day choice
- Cost analysis (CPU and storage)
- Adding new persistent keys with proper TTL handling
