# Trial Escrow Enumeration Design

## Problem
Implementing `expire_trial_offers` requires enumerating all outstanding trial escrows.
A naive unbounded `Vec` would grow forever, making sweep cost proportional to total
historical trial-offer volume rather than current outstanding volume.

## Options Evaluated

| Strategy | Insert | Remove | Iterate | Notes |
|----------|--------|--------|---------|-------|
| Append-only Vec | O(1) | O(1) mark-dead | O(N) all-time | Grows unboundedly; stale entries never pruned |
| Swap-remove Vec | O(1) | O(1) swap-remove | O(N) live-only | Bounded by live entries; compact storage |
| Per-player index | O(1) | O(1) per player | O(N) per player | Natural fit given 24h rate-limit per player |

## Recommendation
**Swap-remove Vec** stored under a single `DataKey::TrialEscrowIndex`.

- Insert: push `(player_id, trial_index)` onto the Vec.
- Remove: swap the target entry with the last element, then pop.
- Iterate: walk the Vec; each entry is live.

Storage cost is bounded by the number of *concurrent* outstanding trial offers,
not historical volume. Given the 24h rate limit per player and typical platform
scale, this remains small and predictable.

## DataKey
```rust
TrialEscrowIndex,  // Vec<(u64, u32)> — outstanding (player_id, trial_index)
```
