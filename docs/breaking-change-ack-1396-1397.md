# Breaking-Change Acknowledgment — issues #1396 / #1397 / #1398

**Branch:** `fix-verify`  
**Script:** `scripts/check-storage-layout-compat.sh main HEAD --acknowledge-breaking-change`

## What changed

- `PendingMilestoneClaim` gains `description_hash: BytesN<32>` and `voters: Vec<Address>` (#1397 / #1398).
- `MilestoneDispute` gains `round` and `resolved_at`; storage key becomes `(player_id, milestone_index, round)` (#1396).
- `DisputeVote` / `DisputeVoteCount` / `OpenDisputeIndex` include round.
- New DataKeys: `DisputeRound`, `PlayerOpenDisputeCount`.

## Migration

1. Run `migrations/008_dispute_rounds.sql` against the indexer DB before activating the new WASM.
2. Resolve or acknowledge open disputes on any already-deployed verification instance before upgrade — prior `MilestoneDispute(player, idx)` entries are not readable under the new key shape.
3. Re-run the storage-layout compat script with `--acknowledge-breaking-change`.
