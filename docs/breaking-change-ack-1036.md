# Breaking-Change Acknowledgment — issue #1036

**Date:** 2026-08-19  
**Branch:** feature/dispute-jury-escalation-1036  
**Script:** `scripts/check-storage-layout-compat.sh main HEAD --acknowledge-breaking-change`

## What was detected

`scripts/check-storage-layout-compat.sh` classifies the following change as
**BREAKING** under the rules in `docs/VERSIONING.md`:

```
BREAKING [verification] MilestoneDispute: struct fields changed
  Removed: (none)
  Added:   impact_score u32
           jury_required bool
           quorum u32
           voting_deadline u64
           votes_for u32
           votes_against u32
```

Adding fields to a `#[contracttype]` struct is a breaking storage-layout change
because the XDR encoding is positional — existing `MilestoneDispute` entries
written by v0.3.0 contracts cannot be decoded by the v0.4.0 ABI without a
migration.

## Acknowledgment

This breaking change is **intentional and expected**, as documented in:

- `CHANGELOG.md` — v1.0.0 entry with migration guides
- `docs/VERSIONING.md` — v1.0.0 version history row (MAJOR)
- `docs/DISPUTE_JURY.md` — design spec (now marked Implemented)

## Migration path

1. **New deployments:** No action required; the contract is deployed fresh.
2. **Upgrades from v0.3.0 (verification) on testnet or mainnet:**
   - Run `migrations/005_dispute_jury.sql` against the indexer PostgreSQL
     database **before** activating the new WASM.
   - Existing `MilestoneDispute` storage entries written by v0.3.0 will be
     unreadable after upgrade. Operators must resolve or acknowledge all
     open disputes on the old contract **before** upgrading, consistent with
     the migration operator checklist in `docs/MIGRATION_GAPS.md` (row 7).
   - The new default field values (impact_score=0, jury_required=false,
     quorum=0, votes_for=0, votes_against=0, voting_deadline=0) are only
     applicable to disputes filed after the upgrade; pre-upgrade dispute
     entries are not retroactively readable.
3. Re-run `scripts/check-storage-layout-compat.sh` with
   `--acknowledge-breaking-change` to unblock the upgrade pipeline.
