# Migration Gaps

> **This is the canonical reference for migration automation coverage.**  
> When any gap listed here is closed by another issue, update the **Status**
> column and add a note rather than deleting the row — the history of what was
> once non-replayable is useful context for future audits.

Two places in the documentation previously mentioned migration gaps in passing:

- [`docs/DEPLOYMENT.md` — "Address migration"](DEPLOYMENT.md#address-migration-new-contract-id)
  describes what `scripts/replay-state.sh` can and cannot replay automatically.
- [`docs/INDEXER.md` — "Known gaps"](INDEXER.md#known-gaps-between-the-contracts-and-this-schema)
  lists places where the PostgreSQL schema does not track a field the contract exposes.

This document consolidates every known non-fully-automatable migration data
category into one place so a migration operator has a single checklist.

---

## Data-category status table

| # | Data category | Contract | Current status | Notes | Tracking issue |
|---|---------------|----------|---------------|-------|---------------|
| 1 | **Validator registrations** | `verification` | ✅ Fully replayable | `replay-state.sh` re-registers every active validator via the admin-only `register_validator` entrypoint. No user action required. | — |
| 2 | **Player profiles** | `registration` | ✅ Fully replayable | `replay-state.sh` exports player payloads via `get_player_count`/`get_player` and re-seeds them through `admin_seed_player`, which bypasses the wallet-auth requirement of `register_player`. The exported payload includes the resolved `level` field from the progress contract. | — |
| 3 | **Scout profiles** | `registration` | ✅ Fully replayable | Same path as players: `get_scout_count`/`get_scout` → `admin_seed_scout`. | — |
| 4 | **Player progress levels** | `progress` | ✅ Fully replayable | The `level` field is captured in the player export (see row 2) and written via `admin_seed_player` on the new registration contract, then synced to the new progress contract by the cross-contract `set_player_level` call that `admin_seed_player` triggers. | — |
| 5 | **Progress history entries** | `progress` | ✅ Fully replayable | `replay-state.sh` exports every old history entry, seeds it in order through `admin_seed_history`, and verifies the old Merkle root on the final entry. | — |
| 6 | **Milestone records** | `verification` | ✅ Fully replayable | `replay-state.sh` replays every milestone through `admin_seed_milestone`, rebuilding evidence, counters, global, and validator indexes. | — |
| 7 | **In-flight milestone disputes** | `verification` | ✅ Fully replayable | `replay-state.sh` enumerates each old milestone's dispute and replays it through `admin_seed_dispute`, rebuilding all dispute indexes and counts. | — |
| 8 | **Scout subscriptions** | `scout_access` | ✅ Fully replayable | `replay-state.sh` replays each active subscription through `admin_seed_subscription`, rebuilding tier and expiry indexes. XLM balances remain on-chain. | — |
| 9 | **Contact records** | `scout_access` | ✅ Fully replayable | `replay-state.sh` enumerates each scout's contacts and replays records through `admin_seed_contact`, rebuilding both reverse indexes. | — |
| 10 | **Trial offers** | `scout_access` | ✅ Fully replayable | `replay-state.sh` replays every offer and reads the escrow getter so in-flight escrow state is preserved through `admin_seed_trial_offer`. | — |
| 11 | **Fee configuration history** | `scout_access` | ✅ Fully replayable | `replay-state.sh` atomically restores the current `FeeConfig` and all bounded history entries through `admin_seed_fee_config`. | — |
| 12 | **Player deactivation state** | `registration` | ✅ Reconciled | The PostgreSQL schema contains `players.deactivated`; the reconciliation tool now compares it against the registration contract's deactivation getter. | — |
| 13 | **Auto-renewal flags** | `scout_access` | ✅ Fully replayable | `replay-state.sh` reads and replays every scout's flag through `admin_seed_auto_renew`. | — |
| 14 | **Milestone pending-re-review flags** | `verification` | ⚠️ Partially replayable | `is_milestone_flagged(player_id, milestone_index)` exposes per-milestone flag state on-chain. `migrations/007_milestone_flags.sql` adds the `milestone_flags` and `revocation_records` tables; the indexer consumes `milestone_flagged`, `milestone_flag_cleared`, and `revocation_cascade_complete` events to keep these tables current. There is no admin-seed path to replay flags onto a new contract; flags are recreated by re-running the bounded revocation cascade. | Closed via #1039 — see [INDEXER.md](INDEXER.md#known-gaps-between-the-contracts-and-this-schema) |

---

## What "status" means

| Status | Meaning |
|--------|---------|
| ✅ Fully replayable | `scripts/replay-state.sh` handles this automatically with no manual steps |
| ⚠️ Partially replayable | Data is readable on-chain but no automated write path exists on the new contract; requires a future admin-seed entrypoint or manual operator action |
| ❌ Not replayable | No automated or admin path exists; requires manual resolution before or after migration |

---

## Migration operator checklist

Before running `scripts/migrate-contract.sh`, verify the following:

1. **Migration exports** — retain and review the timestamped JSON files under
   `migration-export/` after the replay.

2. **Trial escrows** — verify the replay report contains the expected escrow
   records and that `get_trial_escrow` matches the old contract (row 10).

3. **Player deactivations** — run the indexer reconciliation check after cutover
   to confirm `players.deactivated` matches the registration getter (row 12).

4. **Scout subscriptions and auto-renewal** — verify the replay report contains
   the expected active subscriptions and per-scout renewal flags (rows 8 and 13).


---

## How to update this document

This document is a **living index**. When any gap is closed:

1. Change the **Status** to ✅ and update the **Notes** column with the
   entrypoint or mechanism that now handles it.
2. Add a reference to the PR or issue that closed the gap in **Tracking issue**.
3. Do **not** delete the row — historical context matters for future audits.

Cross-linked from:

- [docs/DEPLOYMENT.md — Address migration](DEPLOYMENT.md#address-migration-new-contract-id)
- [docs/INDEXER.md — Known gaps](INDEXER.md#known-gaps-between-the-contracts-and-this-schema)
