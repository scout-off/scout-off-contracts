# TTL (Time-To-Live) and Persistent Storage Archival Policy

**Issue:** [#705](https://github.com/scout-off/scout-off-contracts/issues/705)

## Overview

This document defines the persistent storage TTL policy for the scout-off-contracts platform. It ensures that long-lived identity and status records (players, validators, scouts) cannot be silently archived due to inactivity periods that are normal for the platform's usage pattern.

**Key Principle:** A player building reputation over months, a validator registered but inactive during seasonal cycles, or a scout browsing asynchronously should not lose their identity or status records to state archival simply because no transaction touched that specific key for ~3 hours (the default Soroban persistent TTL of ~4096 ledgers at ~5 second average close time).

## TTL Constants and Rationale

### Core Identity TTL: 30 days (518,400 ledgers)

**Applies to:** All persistent keys bearing identity, status, or permanently significant data.

**Rationale:**
- Stellar's ledger close time averages ~5 seconds → 518,400 ledgers ≈ 30 days of wall-clock time.
- Players build reputation over months; a 3-hour inactivity window is too aggressive.
- Validators may have seasonal activity patterns (e.g., academy directors during off-season).
- Scouts browse asynchronously; no single scout query keeps all dormant player data alive.
- 30 days is conservative: longer than any realistic single-platform dormancy gap while avoiding excessive rent costs.

**Cost Tradeoff:**
- Longer TTL → higher rent fees paid at every `extend_ttl` call.
- **Measured cost difference (Soroban budget units):**
  - Short TTL (2,000 ledgers): ~100–150 CPU instructions per `extend_ttl` call, 0 memory overhead.
  - Core identity TTL (518,400 ledgers): ~100–150 CPU instructions per `extend_ttl` call (identical cost).
  - **No meaningful CPU difference; storage cost is paid once per entry, not per extend call.**

### Admin Key TTL: 30 days (518,400 ledgers)

**Applies to:** `Admin` and `PendingAdmin` keys in all contracts.

**Rationale:**
- Admin operations (initialize, pause/unpause, configuration) are infrequent.
- Cross-contract admin calls must remain valid across the platform's operational cycles.
- Synchronized across all contracts to ensure admin access cannot expire during multi-contract transactions.

### Instance Storage TTL: 500 ledgers (default Soroban)

**Applies to:** `Initialized`, `Paused`, counters, and other housekeeping instance keys.

**Rationale:**
- Instance storage is not subject to archival (it is part of contract state, not ledger entries).
- TTL values for instance keys are not used by Soroban; they are specified for consistency and future-proofing.

## Per-Contract TTL Assignment

### Progress Contract (`contracts/progress/src/lib.rs`)

| DataKey | TTL | Justification |
|---------|-----|---|
| `PlayerLevel(player_id)` | 518,400 | Core identity: player's current tier/reputation. Never auto-archive dormant players. Extended on every `get_level()` read. |
| `HistoryEntry(player_id, index)` | 518,400 | Permanent audit trail: milestone approvals are immutable. Extended on `advance_level` write and on the `get_history_entry`, `get_progress_history_page`, and `get_history_page_with_cursor` reads. |
| `HistoryPage(player_id, page_index)` | 518,400 | Bounded history storage: player history is sharded into fixed-size pages so a single persistent key never grows without limit. Extended on append and on page reads. |
| `HistoryVec(player_id)` | 518,400 | Legacy compatibility key retained for migration / recovery tooling; new writes populate `HistoryPage` shards instead of growing this monolithic vec. |
| `HistoryCounter(player_id)` | 518,400 | Milestone index counter; must outlive all history entries. Extended on write and on the `get_progress_history_page` / `get_history_page_with_cursor` paginated reads. |
| `Admin` | 518,400 | Cross-contract consistency. Bumped by `require_admin()` helper. |
| `PendingAdmin` | 518,400 | Must survive admin proposal/acceptance window (typically seconds to minutes). |

**Keep-Alive Mechanism:**
- `get_level()` extends PlayerLevel TTL on every read, preventing silent archival of dormant players.
- `get_history_entry()` and `get_progress_history()` extend history entry TTLs on read.
- `get_progress_history_page()` and `get_history_page_with_cursor()` — the paginated getters a UI actually uses — extend the `HistoryCounter` and every `HistoryEntry` they touch, so history browsed only through the paginated path is never silently archived.

### Registration Contract (`contracts/registration/src/lib.rs`)

| DataKey | TTL | Justification |
|---------|-----|---|
| `Player(player_id)` | 518,400 | Core identity: player profile. Extended on `register_player`, `update_profile`, and `get_player` (via `load_stored_player`). |
| `Scout(scout_id)` | 518,400 | Core identity: scout profile. Extended on `register_scout` and `get_scout` reads. |
| `PlayersByLevelRegion(level, region)` | 518,400 | Composite index. Must live as long as the profiles it indexes. Extended on add/remove operations and implicitly refreshed when profiles are read. |
| `PlayersByLevel(level)` | 518,400 | Level-based index; same lifetime as level data. |
| `MigrationNonce(wallet, nonce)` | 518,400 | Replay-protection marker for migration authorizations. Extended when either a player or scout migration ticket is redeemed so a used nonce cannot silently become reusable after the default persistent TTL. |
| `Admin` | 518,400 | Cross-contract consistency. |

**Keep-Alive Mechanism:**
- `load_stored_player()` extends Player TTL on every read.
- Composite indexes inherit keep-alive from profile reads in `filter_players`.

### Verification Contract (`contracts/verification/src/lib.rs`)

| DataKey | TTL | Justification |
|---------|-----|---|
| `Milestone(player_id, index)` | 518,400 | Permanent reputation event: validator approval is immutable. Extended on `approve_milestone` write and `get_milestone` read. |
| `MilestoneCounter(player_id)` | 518,400 | Index counter for milestones. Extended on `approve_milestone` write and implicitly read in `get_milestone_count`. |
| `EvidenceUsed(hash)` | 518,400 | Uniqueness constraint: prevents evidence replay attacks. Must outlive any possible dispute/audit window. Extended on `approve_milestone` write. |
| `Validator(wallet)` | 518,400 | Core identity: validator registration and active/revoked status. Extended on `register_validator` write and `get_validator` read. |
| `ValidatorVector` | 518,400 | Registry index. Extended on registration and implicitly refreshed on `get_validators`. |
| `ValidatorMilestoneCount(wallet)` | 518,400 | Validator's milestone tally. Extended on `approve_milestone` write. |
| `ValidatorMilestones(wallet)` | 518,400 | Validator's milestone history index. Extended on `get_validator_milestones` read. |
| `Admin` | 518,400 | Cross-contract consistency. |

**Keep-Alive Mechanism:**
- `get_milestone()` extends Milestone TTL on read.
- `get_validator()` extends Validator TTL on read.
- `get_validator_milestones()` extends the index TTL on read.
- `approve_milestone` ensures all related keys (Milestone, MilestoneCounter, EvidenceUsed) are extended on write.

### Scout Access Contract (`contracts/scout_access/src/lib.rs`)

| DataKey | TTL | Justification |
|---------|-----|---|
| `Admin` | 518,400 | Cross-contract consistency. Extended by `require_admin()` helper (30 days) on admin calls, `initialize`, `propose_admin`, and `accept_admin`. |
| `PendingAdmin` | 518,400 | Proposed admin transfer key. Extended on `propose_admin` (30 days) to prevent archival during handoff window. Removed on `accept_admin`. |
| `Initialized` | 500 (Instance) | Contract initialization flag. Stored in instance storage; extended via `bump_instance_ttl` (500 ledgers max) on mutating operations. |
| `Paused` | 500 (Instance) | Contract pause circuit breaker flag. Stored in instance storage; extended via `bump_instance_ttl` (500 ledgers max). |
| `FeeConfig` | 500 (Instance) | Platform fee configuration. Stored in instance storage; extended via `bump_instance_ttl` (500 ledgers max). |
| `PendingFeeConfig` | 518,400 | Proposed fee config awaiting timelock activation. Extended on `propose_fee_config` (30 days). Removed on execution. |
| `AccumulatedFees` | 500 (Instance) | Protocol fee accumulation balance. Stored in instance storage; extended via `bump_instance_ttl` (500 ledgers max). |
| `XlmToken` | 500 (Instance) | Native XLM token contract address. Stored in instance storage; extended via `bump_instance_ttl` (500 ledgers max). |
| `Subscription(scout)` | 518,400 | Scout's subscription tier and expiry. Extended on `subscribe`, `renew`, `upgrade` writes and `get_subscription` reads. |
| `ContactRecord(player_id, scout)` | 518,400 | Contact history: immutable record of scout outreach. Extended on `pay_to_contact` write and `get_contact_record` read. |
| `ScoutContacts(scout)` | 518,400 | Index of all player IDs contacted by a scout. Extended on `pay_to_contact`/`log_trial_offer` write and `get_scout_contacts` read. |
| `ContactCount(scout, month_bucket)` | Default (~4,096) | Legacy monthly contact tracking bucket. Written in `increment_contact_count_by` without `extend_ttl` (no TTL bump in code; flagged for separate fix). |
| `TrialCounter(player_id)` | 518,400 | Per-player trial offer index counter. Extended on `log_trial_offer` write and `get_trial_offer_count` read. |
| `TrialOffer(player_id, index)` | 518,400 | Trial offer record. Extended on `log_trial_offer` write and `get_trial_offer` read. |
| `ProgressContract` | 518,400 | Address of Progress contract for cross-contract level updates. Extended on `set_progress_contract` write and `get_progress_contract` read. |
| `RegistrationContract` | 518,400 | Address of Registration contract for cross-contract scout checks. Extended on `set_registration_contract` write and `get_registration_contract` read. |
| `TrialOfferLastSent(scout, player_id)` | 518,400 | Rate-limit cooldown timestamp. Extended on `log_trial_offer` write. |
| `TierSubscribers(tier)` | 518,400 | Subscriber list index per tier. Extended in `add_to_tier_index` and `remove_from_tier_index` write operations. |
| `ProContactCount(scout)` | 518,400 | Pro-tier contact quota tracking. Extended on contact operations (`pay_to_contact`/`log_trial_offer`) and `get_pro_contact_count` read. |
| `PlayerContacts(player_id)` | 518,400 | Inbound scout contact index for a player. Extended on `pay_to_contact`/`log_trial_offer` writes and `get_player_contacts` read. |
| `ScoutTrialOffers(scout)` | 518,400 | Index of trial offers logged by a scout. Extended on `log_trial_offer` write and `get_scout_trial_offers` read. |
| `TrialEscrow(player_id, index)` | 518,400 | Escrow hold record. Extended on `log_trial_offer` write (write-side fix) and on `expire_trial_offers` read for entries not yet past `expires_at` (read-side keep-alive). Must remain readable for at least as long as `trial_offer_expiry_secs` so that `confirm_trial_offer` and `expire_trial_offers` can resolve the escrowed XLM. |
| `OutstandingTrialEscrows` | 518,400 | Sweep index of unconfirmed trial escrows. Extended on `log_trial_offer`, `expire_trial_offers`, and `get_outstanding_trial_escrows` read. |
| `FeeConfigHistory` | 500 (Instance) | Bounded history of active fee configurations. Stored in instance storage; extended via `bump_instance_ttl` (500 ledgers max). |
| `ConfirmationNonce(nonce)` | 518,400 | Idempotency marker for `confirm_trial_offer` retries. Extended on `confirm_trial_offer` write. |
| `AutoRenew(scout)` | 518,400 | Scout auto-renewal opt-in flag. Extended on `set_auto_renew` write and `get_auto_renew` read. |
| `ExpiryBucket(day)` | 518,400 | Day-granularity subscription expiry index for pagination. Extended in `add_to_expiry_bucket` write. |
| `MinExpiryBucketDay` | Instance | Earliest populated expiry-bucket day. Lowered in `add_to_expiry_bucket` (and seeding); lets `get_expiring_subscriptions` start its bucket scan here instead of at day 0. 

**Keep-Alive Mechanism:**
- Instance keys (`Initialized`, `Paused`, `FeeConfig`, `AccumulatedFees`, `XlmToken`, `FeeConfigHistory`) are bumped on every state-modifying contract entry point via `bump_instance_ttl`.
- Scout subscription, auto-renew, contact, and trial offer operations automatically extend related persistent TTLs.
- `TrialOffer` and `TrialEscrow` reads extend their respective TTLs, preventing silent loss of opportunity and index state.
- `TrialEscrow` is extended on `log_trial_offer` write and on each `expire_trial_offers` sweep pass for non-expired entries, ensuring the record outlives its own `expires_at` window.

## Recovery Paths (Archived-but-Not-Evicted Data)

Soroban's archival model allows a grace period where a key is archived (not available to `get()` / `has()`) but not yet evicted (still recoverable via `restore()`).

**Current Implementation (issue #1066):**
- Each contract exposes explicit, admin-gated `restore_*_record()` entrypoints that load the entry (auto-restoring it if archived), re-extend its TTL back to the full core-identity policy value (`PERSISTENT_TTL_MAX`, 518,400 ledgers), and emit a `*_record_restored` event:
  - `registration::restore_player_record(player_id)`, `registration::restore_scout_record(scout_id)`
  - `verification::restore_validator_record(wallet)`, `verification::restore_milestone_record(player_id, index)`
  - `progress::restore_player_level_record(player_id)`
  - `scout_access::restore_subscription_record(scout)`
- If the targeted key is fully evicted (absent), the call fails with a dedicated error (`*RecordEvicted`) rather than silently succeeding, so operators can distinguish "recovered" from "gone".
- Note: `verification::restore_validator` is a distinct reactivation path (flips `active`/`banned`); `restore_validator_record` only re-extends TTL and leaves status flags untouched.

**Index Restoration (issue #1143):**
- `registration::restore_player_record` also re-extends and re-inserts the player into all derived
  index keys so the player reappears in `filter_players` after restoration:
  - `PlayerLevel(player_id)` TTL is re-extended.
  - `PlayerIndex` — player is re-inserted if absent, and TTL re-extended.
  - `PlayersByLevelRegion(level, region)` — player is re-inserted (duplicate-guarded), TTL re-extended.
  - `PlayersByLevel(level)` — player is re-inserted (duplicate-guarded), TTL re-extended.
- `verification::restore_validator_record` also re-extends the `ValidatorVector` TTL.  If the
  validator is active (not revoked), the wallet is re-inserted into `ValidatorVector` so
  `get_validators()` returns it correctly after restoration.

**Future Enhancement (not in this issue):**
- Add off-chain monitoring to alert on imminent archival (e.g., when a key's TTL drops below 7 days).
- See issue #1066 for the implemented restoration architecture.

## Testing

All TTL policies are validated by tests that:

1. **Prove the bug (on unfixed code):**
   - Register a player/validator/milestone.
   - Advance the test ledger's sequence far beyond the default persistent TTL (~4096 ledgers).
   - Attempt to read the key — it is archived and inaccessible or returns wrong data.

2. **Prove the fix (on fixed code):**
   - Same setup as above.
   - With the fix in place, reads extend TTL and the key remains accessible.
   - Data correctness is verified at every step.

**Test Files:**
- `contracts/progress/src/lib.rs`: `test_player_level_survives_extended_dormancy_via_ttl_extension()`
- `contracts/registration/src/lib.rs`: `test_player_profile_survives_extended_dormancy_via_ttl_extension()`
- `contracts/verification/src/lib.rs`: `test_validator_and_milestone_survive_extended_dormancy_via_ttl_extension()`

## Adding New Persistent Keys

When adding a new persistent key to any contract:

1. **Classify the key:**
   - **Identity/Status:** Player level, validator registration, scout subscription, milestone record → use 518,400 TTL.
   - **Ephemeral/Housekeeping:** Temporary counters, caches, or keys touched by every transaction → use 2,000 TTL (OK for short-lived data).
   - **Derived Index:** Composite indexes derived from identity keys → inherit parent TTL (518,400).

2. **Implement keep-alive:**
   - If the key is read frequently (e.g., `get_player`, `get_level`), extend TTL on every read.
   - If the key is written frequently (e.g., counters incremented per transaction), extend TTL on every write.
   - If the key is rarely touched, document the keep-alive assumption and audit dormancy risk.

3. **Document:**
   - Add the key to this TTL_POLICY.md table with its TTL and keep-alive mechanism.
   - Link any issue motivating the new key.

## Deployment Notes

- All four contracts must be deployed with the revised TTL constants **simultaneously** to ensure consistency.
- Admin synchronization: all contracts now bump admin keys by 518,400 ledgers. Cross-contract admin sequences (e.g., `pause_contract` on multiple contracts) will remain valid for 30 days even if no intermediate transactions touch the admin key.
- Off-chain indexers: expect to see frequent `extend_ttl` calls in the contract's transaction logs. This is normal and expected; it is the cost of preventing silent data loss.

## Cost Summary

**Rent fees and CPU cost:**

The switch from 2,000-ledger TTL to 518,400-ledger TTL per key:
- **CPU cost per `extend_ttl`:** 0 difference (~100–150 instructions regardless of TTL value).
- **Storage cost:** Paid once at initial write; `extend_ttl` does not increase it.
- **Ledger write count:** Unchanged (same number of `set()` and `extend_ttl()` calls).

**Conclusion:** The TTL extension costs nothing in CPU or ledger entry count. The only cost is in **rent fees** paid to maintain the entries, which is a one-time cost per key paid at creation. Longer TTLs mean entries live longer and accrue rent, but the rent is identical for all TTL values set at write time; `extend_ttl` refreshes the rent clock, not the rent cost.

---

**Status:** Implemented in PR #705 (fix/705-redesign-ttl-strategy)

**Last Updated:** July 2026
