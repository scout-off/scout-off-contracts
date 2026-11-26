# Event Audit Reference

This document lists every on-chain event emitted by the four ScoutChain
contracts, their topic/data schemas, and their indexing obligations. Keep this
file in sync with each contract's `events.rs`.

---

## How events work in Soroban

Each `env.events().publish((topics...), data)` call writes a record to the
transaction's event stream. The Horizon API and Soroban RPC expose these as
`LedgerEntryChange` events under `transaction.result_meta`. Off-chain indexers
(the backend event-stream processor in `scoutchain-backend`) consume these to
keep the Postgres tables in `migrations/001_initial_schema.sql` in sync with
on-chain truth.

---

## Upgrade events

### `contract_upgraded`

Emitted by `upgrade(new_wasm_hash)` in **all four contracts**, *before*
`update_current_contract_wasm` executes, so it is always attributed to the
**old** code version.

| Field  | Value |
|--------|-------|
| Topics | `("contract_upgraded", admin: Address)` |
| Data   | `new_wasm_hash: BytesN<32>` |

**Contracts**: `registration`, `verification`, `progress`, `scout_access`

**Indexer obligation**: Record in `admin_transfers` (or a dedicated
`contract_upgrades` table if added) with the new hash and block timestamp.
Alert on unexpected `contract_upgraded` events in production — an upgrade you
did not initiate is a strong signal of admin key compromise.

**RUNBOOK reference**: See [RUNBOOK.md — Contract Upgrades](RUNBOOK.md#contract-upgrades)
for how to verify the event after running `scripts/upgrade.sh`.

---

## Registration contract events

### `player_registered`
| Field  | Value |
|--------|-------|
| Topics | `("player_registered", wallet: Address)` |
| Data   | `player_id: u64` |

### `scout_registered`
| Field  | Value |
|--------|-------|
| Topics | `("scout_registered", wallet: Address)` |
| Data   | `scout_id: u64` |

### `profile_updated`
| Field  | Value |
|--------|-------|
| Topics | `("profile_updated", wallet: Address)` |
| Data   | `player_id: u64` |

### `scout_verified`
| Field  | Value |
|--------|-------|
| Topics | `("scout_verified", wallet: Address)` |
| Data   | `(scout_id: u64, admin: Address)` |

### `player_deregistered`
| Field  | Value |
|--------|-------|
| Topics | `("player_deregistered", admin: Address)` |
| Data   | `player_id: u64` |

### `scout_deregistered`
| Field  | Value |
|--------|-------|
| Topics | `("scout_deregistered", admin: Address)` |
| Data   | `(scout_id: u64, wallet: Address)` |

### `player_deactivated`
| Field  | Value |
|--------|-------|
| Topics | `("player_deactivated", admin: Address)` |
| Data   | `player_id: u64` |

### `player_reactivated`
| Field  | Value |
|--------|-------|
| Topics | `("player_reactivated", admin: Address)` |
| Data   | `player_id: u64` |

### `player_level_synced`
| Field  | Value |
|--------|-------|
| Topics | `("player_level_synced", caller: Address)` |
| Data   | `player_id: u64` |

### `admin_transfer_proposed` / `admin_transferred`
| Field  | Value |
|--------|-------|
| Topics | `("admin_transfer_proposed"/"admin_transferred", old_admin: Address)` |
| Data   | `new_admin: Address` |

---

## Verification contract events

### `milestone_approved`
| Field  | Value |
|--------|-------|
| Topics | `("milestone_approved", validator: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, description: String, evidence_hash: String)` |

### `validator_specialization_updated`
| Field | Value |
|-------|-------|
| Topics | `("validator_specialization_updated", admin: Address)` |
| Data | `(wallet: Address, old: Vec<String>, new: Vec<String>)` |

**Indexer guidance:** Update the specialization tags associated with `wallet` to the full `new` list, and retain `old` for audit history. This event is emitted only when `set_validator_specializations` succeeds; registration still emits `validator_registered`. Historic events do not provide retroactive tag history.

### `validator_registered`
| Field  | Value |
|--------|-------|
| Topics | `("validator_registered", wallet: Address)` |
| Data   | `credentials: String` |

### `validator_revoked` / `validator_revoked_for_cause`
| Field  | Value |
|--------|-------|
| Topics | `("validator_revoked"/"validator_revoked_for_cause", admin: Address)` |
| Data   | `(wallet: Address, reason: String)` |

### `dispute_resolved`
| Field  | Value |
|--------|-------|
| Topics | `("dispute_resolved", admin: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, upheld: bool)` |

### `player_data_purged`
| Field  | Value |
|--------|-------|
| Topics | `("player_data_purged", admin: Address)` |
| Data   | `(player_id: u64, entries_removed: u32, more: bool)` |

Emitted by `purge_player_data` in the **verification** contract, once per page
of keys removed. `entries_removed` counts the keys removed in that call;
`more == true` means further pages remain for this contract, and
`more == false` marks the final page for the verification contract.

See [PLAYER_ERASURE.md](PLAYER_ERASURE.md) for the full erasure flow.

### Configuration events

These events are emitted by the admin-only configuration setters. Together they
let an auditor reconstruct how the anti-collusion parameters changed over time.
Each event carries the previous and the new value so that the full history can
be rebuilt from the event stream alone.

#### `diversity_config_updated`
| Field  | Value |
|--------|-------|
| Emitted by | `set_diversity_config` |
| Topics | `("diversity_config_updated", admin: Address)` |
| Data   | `(old: DiversityConfig, new: DiversityConfig)` |

**Indexer guidance:** Store the full `new` config and retain `old` for audit
history. See [VALIDATOR_COLLUSION_THREAT_MODEL.md](VALIDATOR_COLLUSION_THREAT_MODEL.md)
for what the diversity parameters control.

> **Note:** `events.rs` also defines a `diversity_config` constant/name that is
> never emitted. Only `diversity_config_updated` appears on-chain; the unused
> name is tracked for cleanup separately.

#### `jury_config_updated`
| Field  | Value |
|--------|-------|
| Emitted by | `set_jury_config` |
| Topics | `("jury_config_updated", admin: Address)` |
| Data   | `(old: JuryConfig, new: JuryConfig)` |

#### `milestone_threshold_updated`
| Field  | Value |
|--------|-------|
| Emitted by | `set_milestone_threshold` |
| Topics | `("milestone_threshold_updated", admin: Address)` |
| Data   | `(old: u32, new: u32)` |

#### `min_region_quorum_updated`
| Field  | Value |
|--------|-------|
| Emitted by | `set_min_region_quorum` |
| Topics | `("min_region_quorum_updated", admin: Address)` |
| Data   | `(old: u32, new: u32)` |

#### `reg_cooldown_updated`
| Field  | Value |
|--------|-------|
| Emitted by | `set_reg_cooldown` |
| Topics | `("reg_cooldown_updated", admin: Address)` |
| Data   | `(old: u64, new: u64)` |

#### `voting_window_secs_updated`
| Field  | Value |
|--------|-------|
| Emitted by | `set_voting_window_secs` |
| Topics | `("voting_window_secs_updated", admin: Address)` |
| Data   | `(old: u64, new: u64)` |

### Diagnostic events

These events explain *why* something did or did not happen — for example why a
milestone did not advance a player's level, or why a batch of votes was
discarded. Indexers should surface them to players and auditors.

#### `level_advancement_deferred`
| Field  | Value |
|--------|-------|
| Topics | `("level_advancement_deferred", player: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, reason: String)` |

**Trigger:** a milestone was approved but the player's level was not advanced
because a gate (diversity, quorum or threshold) was not satisfied. See
[VALIDATOR_COLLUSION_THREAT_MODEL.md](VALIDATOR_COLLUSION_THREAT_MODEL.md).

#### `diversity_gate_unreachable`
| Field  | Value |
|--------|-------|
| Topics | `("diversity_gate_unreachable", player: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, required: u32, available: u32)` |

**Trigger:** the configured diversity requirement cannot be met by the set of
validators that voted on the milestone (e.g. `required` distinct regions exceed
the regions actually represented). See
[VALIDATOR_COLLUSION_THREAT_MODEL.md](VALIDATOR_COLLUSION_THREAT_MODEL.md).

#### `milestone_threshold_unreachable`
| Field  | Value |
|--------|-------|
| Topics | `("milestone_threshold_unreachable", player: Address)` |
| Data   | `(player_id: u64, milestone_index: u32, threshold: u32, active_validators: u32)` |

**Trigger:** the configured approval threshold is greater than the number of
active validators, so the milestone can never reach approval until the
configuration or validator set changes.

#### `validator_votes_invalidated`
| Field  | Value |
|--------|-------|
| Topics | `("validator_votes_invalidated", admin: Address)` |
| Data   | `(validator: Address, invalidated_count: u32)` |

**Trigger:** a validator was revoked and the votes it had cast on pending
milestones were discarded. See
[VALIDATOR_REVOCATION_REREVIEW.md](VALIDATOR_REVOCATION_REREVIEW.md).

#### `batch_revoked`
| Field  | Value |
|--------|-------|
| Topics | `("batch_revoked", admin: Address)` |
| Data   | `(validators: Vec<Address>, reason: String)` |

**Trigger:** a batch revocation of multiple validators completed. See
[VALIDATOR_REVOCATION_REREVIEW.md](VALIDATOR_REVOCATION_REREVIEW.md).

#### `attestation_key_registered`
| Field  | Value |
|--------|-------|
| Topics | `("attestation_key_registered", wallet: Address)` |
| Data   | `(key: BytesN<32>)` |

**Trigger:** a validator registered an attestation key used to sign votes.
Indexers should record the key so that vote signatures can be audited.

> **#1600 limitation:** events emitted immediately before an `Err` return are
> rolled back with the transaction and therefore do not appear in committed
> ledger state. This affects `level_advancement_deferred`,
> `diversity_gate_unreachable` and `milestone_threshold_unreachable` when they
> are emitted on an error path; they can only be observed in raw transaction
> metadata. See issue #1600.

---
*Last updated: 2026-09-27 — added `contract_upgraded` event (issue #1434);
added verification configuration and diagnostic events (issues #1833, #1834).*

## Progress contract events

### `progress_updated`
| Field  | Value |
|--------|-------|
| Topics | `("progress_updated", updated_by: Address)` |
| Data   | `(player_id: u64, old_level: ProgressLevel, new_level: ProgressLevel)` |

`updated_by` always equals the stored `ProgressEntry.updated_by` (issue #1640):
- **Verification path** (`advance_level`, Level 1–2 via `approve_milestone`): the validator wallet that approved the milestone.
- **Scout-access path** (`advance_level_for_scout`, EliteTier via `confirm_trial_offer`): the scout who made the trial offer. The scout_access contract is the authorising party but is not recorded.

### `player_level_reset`
| Field  | Value |
|--------|-------|
| Topics | `("player_level_reset", admin: Address)` |
| Data   | `(player_id: u64, old_level: ProgressLevel, target_level: ProgressLevel)` |

### `player_data_purged`
| Field  | Value |
|--------|-------|
| Topics | `("player_data_purged", admin: Address)` |
| Data   | `(player_id: u64, entries_removed: u32, more: bool)` |

Emitted by `purge_player_data` in the **progress** contract, once per page of
keys removed. `entries_removed` counts the keys removed in that call;
`more == true` means further pages remain for this contract, and
`more == false` marks the final page for the progress contract.

See [PLAYER_ERASURE.md](PLAYER_ERASURE.md) for the full erasure flow.

---

## Scout Access contract events

### `scout_subscribed` (deprecated)

> **Deprecated since v2.0.0, removed in v3.0.0.** Emitted alongside
> `subscription_created`, `subscription_renewed` or `subscription_auto_renewed`
> for backward compatibility only. New consumers must use those events; see
> [VERSIONING.md — Deprecations](VERSIONING.md#deprecations).

| Field  | Value |
|--------|-------|
| Topics | `("scout_subscribed", scout: Address)` |
| Data   | `(tier: SubscriptionTier, fee_paid: i128)` |

### `subscription_created` / `subscription_renewed` / `subscription_auto_renewed`
| Field  | Value |
|--------|-------|
| Topics | `("<event_name>", scout: Address)` |
| Data   | `(tier: SubscriptionTier, subscribed_at: u64, expires_at: u64)` |

`subscription_created` fires on a scout's first subscription, `subscription_renewed`
on a manual renewal or upgrade via `subscribe`, and `subscription_auto_renewed`
when `renew_if_due` renews.

### `player_contacted`
| Field  | Value |
|--------|-------|
| Topics | `("player_contacted", scout: Address)` |
| Data   | `(player_id: u64, fee_paid: i128)` |

### `trial_offer_logged`
| Field  | Value |
|--------|-------|
| Topics | `("trial_offer_logged", scout: Address)` |
| Data   | `player_id: u64` |

### `trial_offer_confirmed`
| Field  | Value |
|--------|-------|
| Topics | `("trial_offer_confirmed", scout: Address)` |
| Data   | `(player_id: u64, index: u32)` |

### `fees_withdrawn`
| Field  | Value |
|--------|-------|
| Topics | `("fees_withdrawn", admin: Address)` |
| Data   | `(to: Address, amount: i128, timestamp: u64)` |

### `player_data_purged`
| Field  | Value |
|--------|-------|
| Topics | `("player_data_purged", admin: Address)` |
| Data   | `(player_id: u64, entries_removed: u32, more: bool)` |

Emitted by `purge_player_data` in the **scout_access** contract, once per page
of keys removed. `entries_removed` counts the keys removed in that call;
`more == true` means further pages remain for this contract, and
`more == false` marks the final page for the scout_access contract.

See [PLAYER_ERASURE.md](PLAYER_ERASURE.md) for the full erasure flow.

### `evidence_access_renewed`
| Field  | Value |
|--------|-------|
| Topics | `("evidence_access_renewed", scout: Address)` |
| Data   | `(player_id: u64, expires_at: u64, fee_stroops: i128)` |

Emitted by `renew_evidence_access`. **Triggered by the scout** (the scout pays
the renewal fee). Extends an expired, non-revoked evidence grant by 90 days and
leaves `has_evidence_access` true. See
[EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md) for how the off-chain key-wrapping
service consumes this event.

### `evidence_access_reinstated`
| Field  | Value |
|--------|-------|
| Topics | `("evidence_access_reinstated", scout: Address)` |
| Data   | `(player_id: u64, player: Address, expires_at: u64)` |

Emitted by `reinstate_evidence_access`. **Triggered by the player** (the player
opts back in). Sets a fresh 90-day expiry and flips `has_evidence_access` back
to true.

### `evidence_revoked_by_player`
| Field  | Value |
|--------|-------|
| Topics | `("evidence_revoked_by_player", scout: Address)` |
| Data   | `(player_id: u64, player: Address)` |

Emitted by `revoke_evidence_access`. **Triggered by the player.** Sets
`revoked = true` and makes `has_evidence_access` false for all future key-wrap
requests. See [EVIDENCE_PRIVACY.md](EVIDENCE_PRIVACY.md) for the privacy
implications.

### `fee_config_proposal_cancelled`
| Field  | Value |
|--------|-------|
| Topics | `("fee_config_proposal_cancelled", admin: Address)` |
| Data   | `(proposal_id: u64)` |

Emitted by `cancel_fee_config_proposal`. **Triggered by the admin.** Clears the
pending fee-config proposal so it can never activate. See
[FEE_CONFIG_PROPOSAL_DESIGN.md](FEE_CONFIG_PROPOSAL_DESIGN.md) for the proposal
lifecycle.

---

## Diagnostic-only events (not committed to ledger)

These events appear only in the **diagnostic stream** — they are emitted
immediately before a transaction-aborting error, so they do not appear in
committed ledger state. Indexers can detect them in raw transaction metadata.

| Event | Contract | Trigger |
|-------|----------|---------|
| `progress_call_failed` | verification, scout_access | Cross-contract `advance_level` fails |
| `progress_contract_not_set` | verification, scout_access | Progress contract not wired |
| `level_advancement_skipped` | verification | Player already at EliteTier |

---

*Last updated: 2026-10-04 — added `evidence_access_renewed`,
`evidence_access_reinstated`, `evidence_revoked_by_player` and
`fee_config_proposal_cancelled` events (issue #1835); added
`player_data_purged` events for progress, verification and scout_access
(issue #1830).*
