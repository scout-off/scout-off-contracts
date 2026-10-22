# Indexer Documentation

## Overview

The indexer watches the Soroban RPC event stream and materializes on-chain state
into PostgreSQL for fast querying.  It runs as a separate process from the API.

## What it checks

| Table | Source | Description |
|-------|--------|-------------|
| `players` | `registration.get_player` / `filter_players` | Player profiles, vitals, IPFS hashes |
| `scouts` | `registration.get_scout` | Scout profiles, region, **verified** flag, **deactivated** flag |
| `contact_records` | `scout_access.player_contacted` events | Contact audit trail |
| `trial_offers` | `scout_access.log_trial_offer` events | Trial offer records |
| `scout_subscriptions` | `scout_access.scout_subscribed` events | Active subscriptions |
| `fee_withdrawals` | `scout_access.fees_withdrawn` events | Fee withdrawal audit log |
| `milestone_flags` | `verification.milestone_flagged` / `milestone_flag_cleared` events | Milestones pending re-review due to for-cause validator revocations |
| `revocation_records` | `verification.validator_revoked_for_cause` / `revocation_cascade_complete` events | Revoked-validator severity + cascade status |
| `evidence_access_grants` | `scout_access.get_player_access_grants` | Per-(player, scout) grants: `granted_at`, `expires_at`, `tier_at_grant`, `revoked`, `revoked_at` |

## How player IDs are enumerated

`registration.get_player_count` returns the **live** count of registered
players (it is decremented by `deregister_player`), while player IDs come from
a monotonically-increasing `PlayerCounter` allocator.  Deriving IDs as
`1..get_player_count()` therefore skips the highest ID after any
deregistration and probes gaps that never existed.

Instead, the reconciler enumerates player IDs as the **union** of:

1. Every `player_id` present in the indexer DB `players` table, and
2. Every `player_id` returned by a paged on-chain enumeration of the
   registration contract's `PlayerIndex`.

This covers every player that exists on-chain or in the DB, including after
deregistrations (gaps in the ID space are simply not probed).  The enumeration
runs whenever any per-player table is selected — `players`, `scouts`,
`contact_records`, `trial_offers`, `scout_subscriptions`, `fee_withdrawals`,
`milestone_flags`, `revocation_records`, or `evidence_access_grants` — so
`--tables evidence_access_grants` reconciles grants for every player.

`--sample <n>` caps the number of enumerated IDs walked and must be a positive
integer; any other value exits with code `2` and an explanatory message.

## Reconciliation

Run `node scripts/reconcile-indexer.js` to compare on-chain state against the
local database.  The script reports:

- Players/scouts present on-chain but missing in the database
- Players/scouts present in the database but missing on-chain
- Field-level mismatches for `players.deactivated`, `scouts.verified`, and `scouts.deactivated`
- Milestone flags: milestones that are flagged on-chain (`is_milestone_flagged` returns `true`) but absent from the `milestone_flags` table, and vice-versa
- Revocation records: validators with a `RevocationRecord` on-chain but no matching row in `revocation_records`
- Evidence access grants: `evidence_access_grants` rows that are present on-chain but missing in the DB (or vice-versa), and field-level mismatches for `granted_at`, `expires_at`, `tier_at_grant`, `revoked`, and `revoked_at`

`player_level_history`, `validator_history`, `fee_config_history`, and
`admin_transfers` are pure event logs with no single "current state" getter
to diff against — reconciling them exactly would mean replaying every
emitted event, which is a different tool. The script documents this
explicitly (it prints them under "Skipped" rather than silently omitting
them) and, for `player_level_history`, cross-checks the per-player row count
against `progress.get_history_count` as a cheap drift signal.

> **Why passing on a distinct empty-vs-missing signal is fine here.**
> `get_history_count` is a *default-on-absent* getter (see
> [`docs/CONTRACT_REFERENCE.md`](CONTRACT_REFERENCE.md#get_history_countplayer_id-u64---u32)):
> it returns `0` both for a registered player with no recorded level changes
> and for a `player_id` that does not exist on the progress contract, with no
> error to tell them apart. For the reconciliation cross-check the ambiguity
> is harmless because existence is disambiguated elsewhere in the same run:
> this loop only iterates players already present in the `player_level_history`
> rows *and* the `players` reconciliation (same sweep) diffs each one against
> `registration.get_player`, reporting a `PlayerNotFound` existence mismatch
> for any row whose player is absent on-chain. Once a player is confirmed to
> exist via the registration contract, a `0` from `get_history_count` means
> exactly "no level changes," which is the only comparison the cross-check
> needs. A distinct `get_history_count` signal (e.g. returning `None` for an
> unknown ID) would only be worth the added complexity if the cross-check ever
> ran in isolation without the registration-contract existence check — it
> currently does not.

- ~~`scouts.verified`~~ — column added in migration `001_initial_schema.sql`
- ~~Player deactivation status~~ — column added in migration `001_initial_schema.sql`
- ~~Milestone pending-re-review flags~~ — `milestone_flags` and `revocation_records` tables added in migration `007_milestone_flags.sql` (issue #1039)
# Indexer Reconciliation

The backend event indexer mirrors on-chain state into PostgreSQL so the
frontend and API can answer queries (player discovery, fee accounting, contact
history) without making live Soroban RPC calls on every request.
`migrations/001_initial_schema.sql` defines the fourteen tables it writes to
(see [README.md — Database Schema](../README.md#database-schema)).

`scripts/reconcile-indexer.js` closes that gap: it queries live contract
state and diffs it against the corresponding Postgres rows, table by table.
Run `node scripts/reconcile-indexer.js` to compare on-chain state against the
local database. The report distinguishes:

- Players/scouts present on-chain but missing in the database
- Players/scouts present in the database but missing on-chain
- Field-level mismatches for the compared fields listed above

### When to run it

- **On a schedule** (recommended: hourly or daily via cron/CI) against
  production, to catch indexer drift before it's noticed by a scout or player.
- **After any indexer deploy or replay**, to confirm the replay landed
  correctly.
- **Whenever `filter_players` results or fee balances look wrong** — this is
  the first diagnostic step before assuming the contract itself is at fault.
- **After a Stellar network incident** (a ledger reorg, an RPC provider
  outage) that might have caused the indexer to process events out of order
  or twice.

### How to run it

Requires:

- Node.js >= 18 and `npm install` run once at the repo root (`pg` is the only
  dependency).
- The [pinned `stellar-cli` version](CONTRIBUTING.md#installing-the-pinned-stellar-cli-version)
  used by `scripts/generate-bindings.sh`, on your `PATH`.
- A `DATABASE_URL` pointing at the backend's Postgres instance. **This
  database lives in the `scoutchain-backend` repo, not here** — this script
  is intentionally standalone and takes the connection string as a parameter
  rather than assuming any deployment.
- The four `*_CONTRACT_ID` variables, either exported directly or available in
  a `.env.contracts` file (the same file `deploy.sh` writes).

```bash
npm install

DATABASE_URL=postgres://user:pass@host:5432/scoutchain \
REGISTRATION_CONTRACT_ID=C... \
VERIFICATION_CONTRACT_ID=C... \
PROGRESS_CONTRACT_ID=C... \
SCOUT_ACCESS_CONTRACT_ID=C... \
  node scripts/reconcile-indexer.js --network testnet
```

Useful options:

| Flag | Purpose |
|------|---------|
| `--network <name>` | Passed to `stellar contract invoke` (default: `testnet`) |
| `--rpc-url <url>` | Enables the `indexer_cursor` ledger-lag check |
| `--source <identity>` | Passed as `--source` to `stellar contract invoke`, if your CLI setup requires one for simulation calls |
| `--sample <n>` | Cap the number of enumerated player/scout IDs walked, for a quick spot-check instead of a full sweep. Must be a positive integer; invalid values exit with code `2` |
| `--tables <a,b,c>` | Check only a subset of tables (see the table above) |
| `--json` | Emit the report as JSON instead of text, for feeding into another tool |

Exit code is `0` for a clean run and `1` when drift is found — wire this into a
scheduled job (cron, CI) and alert on non-zero.

## Known gaps

### Known gaps (resolved)

- ~~`scouts.verified`~~ — column added in migration `001_initial_schema.sql`
- ~~Player deactivation status~~ — column added in migration `001_initial_schema.sql`

### Known gaps between the contracts and this schema

> **For a consolidated list of all migration gaps — including data categories that
> cannot be automatically replayed onto a new contract — see
> [`docs/MIGRATION_GAPS.md`](MIGRATION_GAPS.md).**

The schema gaps that prompted this section have been resolved: the `verified`
column on `scouts` and the `deactivated` column on `players` were added in
`migrations/001_initial_schema.sql`, and the reconciler now ch

/* … truncated 6636 chars — edit only what you need near the top … */
