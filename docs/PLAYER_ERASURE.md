# Player Erasure Design (GDPR Right-to-Erasure)

## Overview

`registration.deregister_player(player_id)` implements the GDPR right-to-erasure
for ScoutChain platform users. Because the platform runs on a public, immutable
blockchain, **erasure has hard limits** that must be disclosed to players before
they request it.

---

## What can and cannot be erased

### Immutable — cannot be erased

| Data | Location | Why |
|------|----------|-----|
| Transaction history (all past calls) | Stellar ledger history (full archive) | Blockchain history is append-only and cannot be modified by anyone, including validators or admin. |
| Emitted events in transaction receipts | Horizon / event stream | Events are part of the transaction XDR; once the ledger closes they are permanent. |
| Stellar account address used to sign transactions | Stellar ledger | An account address is public by design; it cannot be removed from history. |

### Erasable — current-state ledger entries

| Data | Contract | Key(s) | Treatment |
|------|----------|--------|-----------|
| Player profile (wallet, vitals, IPFS hashes) | `registration` | `Player(id)`, `PlayerByWallet(wallet)`, `PlayerLevel(id)`, `PlayerDeactivated(id)` | **Deleted** by `deregister_player` |
| Player index membership | `registration` | `PlayerIndex`, `PlayersByLevel(level)`, `PlayersByLevelRegion(level, region)` | **Removed** by `deregister_player` |
| Progress level | `progress` | `PlayerLevel(id)` | **Deleted** by `purge_player_data` (final page) |
| Progress history entries | `progress` | `HistoryEntry(id, idx)`, `HistoryPage(id, page)` | **Deleted** paged by `purge_player_data` |
| History counter, Merkle root & frontier | `progress` | `HistoryCounter(id)`, `HistoryRoot(id)`, `HistoryVec(id)`, `HistoryFrontier(id)` | **Deleted** by `purge_player_data` (counter and frontier on the final page) |
| Migration cursor | `progress` | `MigrationCursor(id)` | **Deleted** by `purge_player_data` (on the final page) |
| Milestones (description, evidence CID) | `verification` | `Milestone(id, idx)` | **Deleted** paged by `purge_player_data` |
| Milestone disputes | `verification` | `MilestoneDispute(id, idx)`, `PlayerDisputes(id)` | **Deleted** by `purge_player_data` |
| Milestone counter, player affiliations | `verification` | `MilestoneCounter(id)`, `PlayerAffiliations(id)` | **Deleted** by `purge_player_data` |
| Scout contact records (keyed by player) | `scout_access` | `PlayerContacts(id)` | **Deleted** by `purge_player_data` |
| Trial offers and escrows | `scout_access` | `TrialOffer(id, idx)`, `TrialEscrow(id, idx)`, `TrialCounter(id)` | **Deleted** by `purge_player_data` (only after escrows are expired/refunded) |
| Evidence access grants | `scout_access` | `EvidenceAccessGrant(id, scout)`, `EvidenceAccessGrantPage(id, page)`, `EvidenceAccessGrantCount(id)` | **Deleted** by `purge_player_data` |

### Retained by design (tombstones)

| Data | Reason |
|------|--------|
| Player ID counter monotonic value | IDs must never be reused after erasure to prevent `ContactRecord` collisions (see issue #234). The counter itself is not erased. |
| Validator `MilestoneRef` entries in `ValidatorMilestones(validator)` | Keeps the validator's per-player milestone count intact for audit; the underlying `Milestone` record itself is erased so no PII is accessible via this index. |
| Scout `ContactRecord(player_id, scout)` individual entries | Retains the scout's paid-for contact event for billing audit. The `PlayerContacts(player_id)` reverse index is erased. |
| **Erasure tombstone flag** | `DataKey::Erased(player_id)` written by `registration.deregister_player` to each contract. Once set, all player-scoped writes are permanently blocked with `PlayerErased` error (issue #1622). |

---

## Erasure tombstone invariant (issue #1622)

### Problem

Between and after the paged purge calls, nothing prevents state for the same `player_id` from being written again:
- `progress.admin_seed_history` during a migration window
- `restore_*` record entrypoints
- `verification` approval paths when registration is unwired (require_active_player fails open)
- `scout_access` writes for a player whose registration lookup is not performed on that path

### Solution: Push-model tombstone

When `registration.deregister_player` completes, it cross-calls each wired contract's `mark_player_erased(player_id)` function. This writes a local `Erased(player_id)` tombstone flag in persistent storage with long TTL (same as core identity data).

Each contract then checks this flag in **all player-scoped write entrypoints**:
- **progress**: `advance_level`, `advance_level_for_scout`, `reset_player_level`, `admin_seed_history`, `restore_player_level_record`
- **verification**: `approve_milestone`, `attest_milestone`, `submit_attested_milestone`, `dispute_milestone`, `admin_seed_milestone`, `admin_seed_dispute`, `restore_milestone_record`
- **scout_access**: `pay_to_contact`, `batch_contact_players`, `log_trial_offer`, `admin_seed_contact`, `admin_seed_trial_offer`

If the tombstone is set, the entrypoint returns `PlayerErased` error.

### Why push instead of pull?

A pull design (each contract calling `registration.is_erased(player_id)` on every write) would:
- Add cross-contract call overhead to hot paths
- Fail open when registration is unwired (the check would be skipped)

The push design:
- Avoids per-write cross-contract calls (O(1) local check)
- Fails closed even if registration is later unwired (tombstone is local)
- Is idempotent (calling `mark_player_erased` multiple times is safe)

### Invariant

**Once erasure starts, no contract accepts writes for this `player_id` again.**

This invariant is enforced by the combination of:
1. `deregister_player` pushing tombstones to all wired contracts
2. Each contract checking the tombstone before any player-scoped write
3. The tombstone persisting with long TTL (same as core identity data)

### Error codes

| Contract | Error | Code |
|---------|-------|------|
| `registration` | `PlayerErased` | 22 |
| `progress` | `PlayerErased` | 30 |
| `verification` | `PlayerErased` | 66 |
| `scout_access` | `PlayerErased` | 61 |

---

## Recommended erasure procedure

Run the following steps **in order**. Each cross-contract purge must complete
(`more == false`) before moving to the next step.

### Guards on `purge_player_data`

All three `purge_player_data` functions (`scout_access`, `verification`,
`progress`) share the same guard set and order:

1. `require_admin` — erasure is admin-only.
2. `require_initialized` — the contract must be initialized.
3. `bump_instance_ttl` — instance storage TTL is extended.

Erasure is **allowed while the contract is paused**. Pausing stops user
activity, not compliance operations: an operator told to stop processing
personal data during an incident must still be able to complete erasure.
Purge calls therefore do not call `require_not_paused`. This is an intentional
exception to the project's `require_not_paused → require_initialized` guard
ordering convention and is recorded in `scripts/check-guard-ordering.sh`.

If the escrow-refund path is added to `scout_access.purge_player_data`, note
explicitly that refunds may occur while the contract is paused, so that
erasure is never blocked by outstanding escrows during a pause.

```
1. Expire or refund all outstanding trial escrows for the player.
   Call:  scout_access.expire_trial_offers(limit)
   Until: no escrow remains for this player.

2. Purge scout_access data (paged).
   Call:  scout_access.purge_player_data(player_id, cursor=0)
   Until: more == false.

3. Purge verification data (paged).
   Call:  verification.purge_player_data(player_id, cursor=0)
   Until: more == false.

4. Purge progress data (paged).
   Call:  progress.purge_player_data(player_id, cursor=0)
   Until: more == false.

5. Deregister the player from registration (removes profile + indexes).
   Call:  registration.deregister_player(player_id)
```

Each `purge_player_data` call processes at most 50 paged entries, plus any
scalar keys on the final page, to stay within per-transaction CPU/memory
budgets. Cursors are independent per contract:
start each purge at `cursor=0` and pass that contract's returned cursor to the
next call. The contract stores the expected cursor and rejects a wrong, stale,
or out-of-range value with `InvalidPurgeCursor`; do not restart a partially
completed purge at zero. Scalar keys are removed on the final page, so an
incorrect cursor cannot skip them while reporting completion.

---

## Outstanding escrow guard

`scout_access.purge_player_data` refuses to proceed if any unexpired, non-zero
trial escrow exists for the player, returning the dedicated
`ScoutAccessError::TrialEscrowOutstanding` error (not `TrialOfferNotFound` —
the offer exists, but its escrow is still outstanding). This protects scouts
whose escrowed funds would otherwise be silently destroyed. The operator must
call `expire_trial_offers` or wait for the escrow window to elapse before
erasure can proceed.

This guard applies regardless of the pause state: erasure is permitted while
paused, but outstanding escrows must still be expired or refunded first. If
refunds are performed as part of the purge path, they may occur while the
contract is paused.

---

## Legal / product notes

1. **Inform players before erasure** that their on-chain transaction history
   (Stellar ledger) is permanently public and cannot be deleted.
2. **Erasure is irreversible.** Once a player ID's current-state entries are
   deleted, the profile cannot be reconstructed from on-chain data.
3. **Player IDs are tombstoned.** After `deregister_player` the ID is never
   reassigned, preventing stale data collisions.
4. **This doc requires legal sign-off** before being presented to users as a
   GDPR compliance statement. The platform's legal team should verify it meets
   the applicable regulation's "right to be forgotten" requirements given the
   immutability constraints above.

---

## Database / indexer handling

The PostgreSQL indexer must also scrub the erased player's rows:

```sql
-- After receiving a player_deregistered event:
DELETE FROM players WHERE player_id = $1;
DELETE FROM player_level_history WHERE player_id = $1;
DELETE FROM milestones WHERE player_id = $1;
DELETE FROM milestone_disputes WHERE player_id = $1;
DELETE FROM contact_records WHERE player_id = $1;
DELETE FROM trial_offers WHERE player_id = $1;
DELETE FROM evidence_access_grants WHERE player_id = $1;
-- Retain scout_subscriptions (scout data, not player data).
```

The reconciliation script (`scripts/reconcile-indexer.js`) should treat a
`player_deregistered` event as a hard delete signal, not a diff — it must
remove the player row even if the on-chain state no longer has a matching
record (because the key was erased).

Each `purge_player_data` call on `progress`, `verification` and `scout_access`
also emits a `player_data_purged` event (`admin`, `player_id`,
`entries_removed`, `more`). Indexers should use it to scrub rows incrementally:
the event is emitted once per purge page, `entries_removed` counts the keys
removed in that call, and `more == false` signals the final page for that
contract. See the `player_data_purged` entry in `docs/EVENT_REFERENCE.md` for
the exact topics and data layout of each emitting contract.

See `docs/INDEXER.md` for the indexer cursor model.
