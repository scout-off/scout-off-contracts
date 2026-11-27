# Player Erasure Procedure (GDPR Right to Erasure)

This document describes how operators can prove a player's data is fully purged across all ScoutChain contracts. Each of the three purging contracts (`progress`, `verification`, `scout_access`) persists erasure progress on-chain, emits explicit start and completion events, and exposes a `get_purge_status` view. The `registration` contract exposes an optional aggregator view.

---

## Overview

A complete erasure for `player_id` requires running `purge_player_data` on three contracts until each reports `completed: true`. Because Soroban transactions are bounded in CPU budget, the function removes data in pages and returns a cursor for the next call. The cursor is **stored on-chain** — callers never need to track it themselves.

```
for each of: progress, verification, scout_access
  loop:
    (next_cursor, more) = contract.purge_player_data(admin, player_id, 0)
    while more:
      (next_cursor, more) = contract.purge_player_data(admin, player_id, next_cursor)
```

After all three loops complete, call `registration.get_erasure_status(player_id)` to confirm `fully_erased: true`.

---

## Per-Contract Behaviour

### `progress`

| Page | Keys removed |
|------|-------------|
| 0    | `PlayerLevel`, `HistoryCounter`, `HistoryRoot`, `HistoryVec` |
| 1…n  | `HistoryEntry(player_id, i)` and `HistoryPage` shards, 20 per page |

### `verification`

| Page | Keys removed |
|------|-------------|
| 0    | `MilestoneCounter`, `PlayerDisputes`, `PlayerAffiliations` |
| 1…n  | `Milestone(player_id, i)` + `MilestoneDispute(player_id, i)`, 20 per page |

### `scout_access`

| Page | Keys removed |
|------|-------------|
| 0    | `ContactRecord` entries (one per scout), `PlayerContacts`, `EvidenceAccessGrantCount` |
| 1…n  | `EvidenceAccessGrantPage` shards + individual `EvidenceAccessGrant` records |

---

## Cursor Safety

The `cursor` parameter you supply must equal the stored cursor. Passing the wrong value (e.g. starting at 50) is rejected with an error — no data is skipped silently. Always pass `0` on the first call; on subsequent calls pass the `next_cursor` from the previous response.

---

## On-Chain Status

```rust
// On each purging contract:
let status: PurgeStatus = contract.get_purge_status(player_id);
// { started: bool, cursor: u32, completed: bool }

// Aggregate across all three contracts:
let aggregate: ErasureStatus = registration.get_erasure_status(player_id);
// { progress, verification, scout_access, fully_erased }
```

`PurgeCompleted(player_id)` is stored with a long TTL (518 400 ledgers ≈ 30 days) so auditors can confirm erasure without replaying event history.

---

## Events Emitted

| Event | When emitted |
|-------|-------------|
| `player_purge_started` | First page of `purge_player_data` for this player |
| `player_data_purged` | Every page (topics: `admin`; data: `(player_id, entries_removed, next_cursor, more)`) |
| `player_purge_completed` | Final page when no data remains |

All three events are emitted on `progress`, `verification`, and `scout_access`. See [EVENT_REFERENCE.md](EVENT_REFERENCE.md) for full topic/data schemas.

---

## Step-by-Step Operator Procedure

1. **Confirm the player exists** via `registration.get_player(player_id)`.
2. **Purge `progress`**: call `progress.purge_player_data(admin, player_id, 0)`. Repeat with the returned cursor while `more == true`.
3. **Purge `verification`**: same pattern on the verification contract.
4. **Purge `scout_access`**: same pattern on the scout_access contract.
5. **Deregister the profile**: call `registration.deregister_player(player_id)` (admin-only).
6. **Confirm**: call `registration.get_erasure_status(player_id)` and verify `fully_erased: true`.

---

## Configuring the Aggregator

For step 6 to work, wire the three peer addresses into the registration contract once after deployment:

```bash
stellar contract invoke --id $REGISTRATION_ID -- set_progress_contract \
  --addr $PROGRESS_ID

stellar contract invoke --id $REGISTRATION_ID -- set_verification_contract \
  --addr $VERIFICATION_ID

stellar contract invoke --id $REGISTRATION_ID -- set_scout_access_contract \
  --addr $SCOUT_ACCESS_ID
```

---

## Resumability

If a purge is interrupted (e.g. the operator's script crashes), it resumes from the stored cursor on the next call. `get_purge_status` returns `{ started: true, cursor: N, completed: false }` for an in-progress purge.

---

## ABI Notes

`purge_player_data(admin: Address, player_id: u64, cursor: u32) -> (u32, bool)`

The `cursor` parameter is validated against the stored value. Supplying `0` on the first call and the returned `next_cursor` on subsequent calls is the correct usage pattern.
