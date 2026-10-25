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
| Progress level | `progress` | `PlayerLevel(id)` | **Deleted** by `purge_player_data` (cursor=0) |
| Progress history entries | `progress` | `HistoryEntry(id, idx)`, `HistoryPage(id, page)` | **Deleted** paged by `purge_player_data` |
| History counter & Merkle root | `progress` | `HistoryCounter(id)`, `HistoryRoot(id)`, `HistoryVec(id)` | **Deleted** by `purge_player_data` |
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

---

## Recommended erasure procedure

Run the following steps **in order**. Each cross-contract purge must complete
(`more == false`) before moving to the next step.

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

Each `purge_player_data` call removes at most 50 entries to stay within
per-transaction CPU/memory budgets. For a player with a long history, multiple
calls with the returned cursor value are needed.

---

## Outstanding escrow guard

`scout_access.purge_player_data` refuses to proceed if any unexpired, non-zero
trial escrow exists for the player. This protects scouts whose escrowed funds
would otherwise be silently destroyed. The operator must call
`expire_trial_offers` or wait for the escrow window to elapse before erasure
can proceed.

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

See `docs/INDEXER.md` for the indexer cursor model.
