# Indexer Documentation

## Overview

The indexer watches the Soroban RPC event stream and materializes on-chain state
into PostgreSQL for fast querying.  It runs as a separate process from the API.

## What it checks

| Table | Source | Description |
|-------|--------|-------------|
| `players` | `registration.get_player` / `filter_players` | Player profiles, vitals, IPFS hashes |
| `scouts` | `registration.get_scout` | Scout profiles, region, **verified** flag |
| `contact_records` | `scout_access.player_contacted` events | Contact audit trail |
| `trial_offers` | `scout_access.log_trial_offer` events | Trial offer records |
| `scout_subscriptions` | `scout_access.scout_subscribed` events | Active subscriptions |
| `fee_withdrawals` | `scout_access.fees_withdrawn` events | Fee withdrawal audit log |
| `milestone_flags` | `verification.milestone_flagged` / `milestone_flag_cleared` events | Milestones pending re-review due to for-cause validator revocations |
| `revocation_records` | `verification.validator_revoked_for_cause` / `revocation_cascade_complete` events | Revoked-validator severity + cascade status |

## Reconciliation

Run `node scripts/reconcile-indexer.js` to compare on-chain state against the
local database.  The script reports:

- Players/scouts present on-chain but missing in the database
- Players/scouts present in the database but missing on-chain
- Field-level mismatches for `players.deactivated` and `scouts.verified`
- Milestone flags: milestones that are flagged on-chain (`is_milestone_flagged` returns `true`) but absent from the `milestone_flags` table, and vice-versa
- Revocation records: validators with a `RevocationRecord` on-chain but no matching row in `revocation_records`

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
| `--sample <n>` | Cap the number of player/scout IDs walked, for a quick spot-check instead of a full sweep |
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
`migrations/001_initial_schema.sql`, and the reconciler now checks
`scouts.verified` against the on-chain value. The remaining known difference is
`contact_records.contacted_at`, which records the indexer's insert time rather
than the contract's ledger timestamp and is therefore checked for existence
only (see the table above).

## What to do when drift is found

1. **Re-run with `--sample`** on the affected table to confirm the drift is
   reproducible and not a transient RPC hiccup.
2. **Check `indexer_cursor` first** — if the indexer is far behind the latest
   ledger, most "mismatches" are just events it hasn't processed yet, not real
   bugs. Wait for it to catch up and re-run.
3. **For a handful of rows**: manually re-derive the correct value from the
   contract and issue a targeted `UPDATE` in the backend's indexer, then
   confirm the fix with `--tables <table> --sample <n>` scoped to the affected
   IDs.
4. **For widespread drift across a table**: treat it as an indexer bug — check
   the backend's event-processing logs around when the drift likely started,
   and consider a full replay of that table from the on-chain event log rather
   than patching rows by hand.
5. **Escalate** to whoever owns the `scoutchain-backend` indexer if the cause
   isn't obvious from the mismatch detail — the report includes the exact key,
   field, on-chain value, and off-chain value needed to start that
   investigation.

## Recent improvements (issue #1015)

The following correctness fixes shipped on `fix/1015-indexer-correctness`:

### 1. Event pagination (audit-event-history.js)

**Problem:** `fetchEvents` used a hardcoded `limit: 10000` with no cursor/continuation logic, silently dropping all events beyond the first 10,000.

**Fix:** Replaced the single RPC call with a cursor-based pagination loop, using `EVENTS_PAGE_SIZE = 200` per page (the maximum the RPC accepts) and looping until a page smaller than 200 is returned, signaling the end of the event stream.

**Impact:** Before this fix, any player or scout with more than 10,000 total events across all contracts would have had their later events silently omitted from the audit, causing both missed milestone/subscription events and incorrect internal event-chain consistency checks. Now all events are retrieved across all pages.

### 2. Reorg detection (audit-event-history.js)

**Problem:** After sorting events by ledger sequence, the tool assumed chronological order was correct and never checked whether the RPC had delivered events out of order — which can happen when a ledger reorg rolls back some ledgers and re-delivers their events interleaved with newer ones.

**Fix:** After the sort, the tool now walks the full event array and flags any event whose ledger sequence is numerically lower than the previous event's. Each out-of-order pair is logged as a `warning`-severity reorg issue, and a single `error`-severity summary issue is added so the audit exits with status 1, signaling that the reconstructed state may be unreliable.

**Impact:** Operators now have a clear signal when the event stream contains a reorg, rather than silently trusting reconstructed state that may be incorrect. The tool still processes the events (after sorting) but marks the audit as failed so the operator knows to investigate.

### 3. Subscription tier divergence (reconcile-indexer.js)

**Problem:** The subscription reconciler compared `tier`, `subscribed_at`, and `expires_at` but did not detect:

- **Expired-but-active divergence**: A subscription that has expired on-chain (current time > `expires_at`) but whose DB row still shows the scout as active, incorrectly granting the scout access they no longer have.
- **Missing auto-renewal flag**: The `auto_renew` per-scout opt-in was never reconciled, so a scout who opted in on-chain but whose DB row was not updated would appear to have no auto-renewal configured.
- **DB-only subscriptions**: A scout subscription row in the DB that has no on-chain counterpart at any tier (e.g., the transaction was rolled back or the contract was upgraded and the indexer replay missed it).

**Fix:** 

1. Added an `active_state` check: after fetching the subscription from the contract, compare `expires_at` against the current Unix timestamp. If the subscription is expired on-chain but the DB `expires_at` has not been updated to reflect that (or the row was not deleted), flag it as `active_state` mismatch.

2. Added `auto_renew` flag reconciliation: call `scout_access.get_auto_renew(scout)` and compare the result against `scout_subscriptions.auto_renew` (if the column exists).

3. Added a sweep at the end of the reconciler: after collecting all scouts known to the contract across all three tiers, walk the DB rows and flag any scout that exists in the DB but was not found under any tier on-chain.

**Schema change:** Added `auto_renew BOOLEAN NOT NULL DEFAULT FALSE` to the `scout_subscriptions` table via `migrations/004_scout_subscriptions_auto_renew.sql`.

**Impact:** The reconciler now catches three classes of subscription drift that were previously invisible: stale active-but-expired subscriptions, missing auto-renewal flags, and DB-only rows with no on-chain state. This closes a correctness gap where scouts could appear to have valid subscriptions in the off-chain index when the contract would reject their `pay_to_contact` or trial-offer calls.

### 4. `reconcile-indexer.js` did not parse

**Problem:** The file contained an entire stale placeholder implementation (an early stub with hardcoded `fetchPlayersFromChain`/`fetchScoutsFromChain` returning `[]`) concatenated ahead of the real implementation, left over from a previous merge. The stub's `main().catch((err) => {` was never closed, so the file was a syntax error end-to-end — `node --check scripts/reconcile-indexer.js` failed, meaning the reconciler (and therefore every fix in this section) could not actually run.

**Fix:** Removed the dead stub (the placeholder `connectDb`/`closeDb`/`fetchPlayersFromChain`/`fetchScoutsFromChain`/`reconcilePlayers`/`reconcileScouts`/`main` and their unclosed `main().catch(...)`), keeping only the real, fully-implemented reconciler that follows it.

**Impact:** `node --check scripts/reconcile-indexer.js` now passes. This was a pre-existing bug on `main`, unrelated to but blocking the fixes above.

## Related documentation

- [RUNBOOK.md](RUNBOOK.md) — emergency pause/unpause and other operational
  procedures.
- [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md) — full getter reference for
  every function this script calls.
- [README.md — Database Schema](../README.md#database-schema) — the
  Postgres schema this script reconciles against.
