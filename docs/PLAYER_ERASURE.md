# Player erasure procedure (GDPR right-to-erasure)

This document is the **single source of truth** for how a player's on-chain
data is erased across the four ScoutChain contracts, and for the keys that are
intentionally **retained**. The property-based completeness test in
`contracts/chaos-tests/tests/erasure_completeness.rs` mirrors the "Retained"
table here verbatim as its allow-list: if a key is listed as retained below it
is permitted to survive erasure, and nothing else keyed by the player is.

## Contracts in scope

| Contract        | Erasure entrypoint                          |
|-----------------|---------------------------------------------|
| `registration`  | `purge_player_data(env, player_id, cursor)`  |
| `progress`      | `purge_player_data(env, player_id, cursor)`  |
| `verification`  | `purge_player_data(env, player_id, cursor)`  |
| `scout_access`  | `purge_player_data(env, player_id, cursor)`  |

Every entrypoint is **admin-only** and returns `PurgePage { next_cursor, more }`
(`scoutchain_shared_types::PurgePage`). The erasure driver loops, feeding
`next_cursor` back into the next call, until `more` is `false`.

## Procedure

Run the four purges **to completion** for the target `player_id`. Order does
not matter for completeness — each contract only touches its own storage —
but the recommended order below keeps token conservation easy to reason about
(scout_access refunds outstanding trial escrows first).

```
let mut cursor = 0u32;
loop {
    let page = scout_access.purge_player_data(&player_id, &cursor)?;
    cursor = page.next_cursor;
    if !page.more { break; }
}
// repeat the same cursor loop for verification, progress, and registration
```

### Token-balance conservation

Before / during erasure no tokens are minted or burned:

* `scout_access.purge_player_data` **refunds** any outstanding (unconfirmed,
  unexpired) trial escrow for the player back to the originating scout before
  dropping the `TrialEscrow` record, so escrowed XLM is never orphaned in the
  contract.
* Already-collected contact and subscription fees stay in
  `AccumulatedFees` untouched (they are platform revenue, not player funds).
* `withdraw_fees` is not part of the procedure.

The test asserts the total XLM supply across all participants and contract
accounts is invariant across the procedure.

## Retained keys (the allow-list)

These keys **may** remain after a complete erasure. They are keyed by the
player and are kept for the stated reason. Everything else keyed by the
player is removed.

| Contract        | DataKey                                              | Reason                                                          |
|-----------------|------------------------------------------------------|-----------------------------------------------------------------|
| `scout_access`  | `ContactRecord(player_id, scout)`                    | Billing / contact audit trail (who contacted the player).       |
| `scout_access`  | `PlayerContacts(player_id)`                          | Inbound scout index for the retained contact records.           |
| `scout_access`  | `EvidenceAccessGrant(player_id, scout)`              | Append-only evidence-access audit trail (see EVIDENCE_PRIVACY). |
| `scout_access`  | `EvidenceAccessGrantCount(player_id)`               | Pagination counter for the retained grants.                    |
| `scout_access`  | `EvidenceAccessGrantPage(player_id, page)`           | Pagination index for the retained grants.                      |

### Retained-by-design (not keyed by the player)

These structures are global / per-validator audit structures that are **not**
keyed by `player_id`, so the completeness oracle (which decodes each `DataKey`
and checks whether it embeds the erased `player_id`) never flags them. The
purge still removes the player's *references* from them where feasible:

* `verification` `GlobalMilestoneSlot(slot)` — bounded ring buffer; a player's
  milestone entries are overwritten as new milestones are approved globally.
* `verification` `OpenDisputeIndex` — global list of open disputes; the purge
  removes the player's tuples.
* `verification` `ValidatorMilestones(validator)`, `ValidatorPlayers(validator)`,
  `ValidatorMilestoneCount(validator)`, `ValidatorPlayerMilestoneCount(validator, player_id)` —
  per-validator indexes; the purge removes the player's references.
* `verification` `EvidenceUsed(evidence_hash)` — global evidence-uniqueness
  map; the purge drops the player's milestone entries.
* `scout_access` `ScoutContacts(scout)`, `ScoutTrialOffers(scout)`,
  `OutstandingTrialEscrows` — scout-centric / global indexes; the purge removes
  the player's references.

### Out of scope (not player-keyed, wallet-scoped)

These keys are keyed by a **wallet address**, not a `player_id`, so they are
neither purged nor flagged by the oracle. A full wallet-scoped erasure (e.g.
for an account-closure workflow) would address them separately:

* `registration` `PlayerRegLastSent(wallet)`, `MigrationNonce(wallet, nonce)`
* `scout_access` `Subscription(scout)`, `ScoutRegLastSent(wallet)`,
  `ContactCount(scout, month)`, `ProContactCount(scout)`, `AutoRenew(scout)`,
  `ExpiryBucket(day)`, `TierSubscribers(tier)`, `ConfirmationNonce(nonce)`

## Known pre-fix bugs this procedure closes

These are the gaps a hand-written per-contract purge left on `main` and that
the completeness test would have caught. Each is closed by the
`purge_player_data` implementation above.

* **scout_access** removed `TrialCounter` on the first page and stopped, so
  later pages purged nothing — outstanding escrows orphaned XLM and trial
  offers / cooldown markers lingered.
* **progress** left `HistoryRoot`, `HistoryPage` shards and `PlayerLevel`
  behind after deleting history entries.
* **verification** left `DisputeVote`, `DisputeVoteCount`, `MilestoneDispute`,
  `PlayerDisputes`, `PendingMilestoneVote`, `PendingMilestoneClaim`,
  `MilestoneCounter`, `Milestone`, `PlayerAffiliations`, and the player's
  entries in the per-validator and global indexes.
* **registration** `deregister_player` left `PlayerDeactivated` and the
  per-level `PlayersByLevel` index entry behind.
