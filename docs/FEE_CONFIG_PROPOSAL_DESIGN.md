# Fee Configuration Propose-Then-Activate Mechanism (Issue #807)

> **Status: Implemented.** `propose_fee_config` and `activate_fee_config` ship
> in the `scout_access` contract with error codes 24–26
> (`NoPendingFeeConfig`, `FeeConfigProposalNotReady`,
> `PendingFeeConfigAlreadyExists`) and the `fee_config_proposed` event. See
> [CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access) for the live
> function reference. The one remaining gap is the proposal-cancellation event
> (see [Rollback and Edge Cases](#rollback-and-edge-cases)), tracked separately.

## Overview

This document specifies the timelocked propose-then-activate mechanism for fee changes in the scout_access contract, providing scouts with on-chain-enforced advance notice before any fee increase takes effect.

`update_fee_config` takes effect atomically in the same transaction it's called in, with zero advance notice. This contrasts sharply with the existing two-step `propose_admin` / `accept_admin` pattern already used for admin rotation, which gives affected parties a chance to react.

The `propose_fee_config` and `activate_fee_config` functions close that gap.

---

## Design Decisions

### 1. Delay Model: Fixed Contract Constant

**Decision:** The activation delay is a **fixed contract constant** (not admin-configurable).

**Rationale:**
- Scouts need *predictable* advance-notice guarantees; an admin-configurable delay would shift risk to scouts (admin could reduce the delay on short notice).
- A fixed constant is easier to audit, test, and document in on-chain SLAs.
- Fee increases are rare; a fixed delay is not a burden on administrative operations.

**Constant Value:** `FEE_CONFIG_PROPOSAL_DELAY_SECS = 7 * 24 * 60 * 60` (7 days)
- Aligns with Stellar's typical governance and operational cycles.
- Scouts have a full week to decide whether to renew, upgrade, or exit their subscription before a new fee structure takes effect.

### 2. In-Flight Transaction Semantics: Use Current Active Config

**Decision:** Calls to `subscribe`, `pay_to_contact`, and `log_trial_offer` landing after a fee change is proposed but before it's active **must use the old (currently active) config**, never a merely-proposed one.

**Rationale:**
- Scouts have already signed their intent based on published fees.
- Atomicity: if a scout's transaction lands between proposal and activation, they should pay the fee they signed up for.
- No ambiguity in which config applies: "the one that was active when this transaction was submitted".

**Implementation:**
- `subscribe`, `pay_to_contact`, and `log_trial_offer` always call `Self::fee_config(&env)`, which returns the *active* config from `DataKey::FeeConfig`.
- A proposed config is stored separately in `DataKey::PendingFeeConfig` and is never consulted by business logic.
- Only `activate_fee_config` moves a proposed config from `PendingFeeConfig` to `FeeConfig`.

### 3. Fee Decreases: No Delay Required

**Decision:** Fee *decreases* do **not require advance notice**. The proposal/activation mechanism applies only to **increases** and **neutral changes** (some fees up, some down, or same total cost).

**Rationale:**
- A decrease benefits scouts immediately; there is no downside to fast-track activation.
- Scouts who have already committed to subscriptions at the higher rate benefit sooner from the decrease.
- Proposal/activation introduces TTL overhead; applying it to decreases is wasteful.

**Implementation Detail:**
- `propose_fee_config` checks whether any individual fee increases relative to the active config. If all are decreases (contact fee, basic/pro/elite tiers all lower than the active config), it immediately activates and emits both `fee_config_proposed` and `fee_config_updated` events in the same transaction.
- If there is any increase, the config is stored as pending, activation is delayed, and only `fee_config_proposed` is emitted.

---

## Data Structures and Storage

### New DataKey Variant

```rust
#[contracttype]
pub enum DataKey {
    // ... existing keys ...
    /// Proposed fee config awaiting activation after delay; paired with PendingFeeConfigProposedAt
    PendingFeeConfig,
    /// Timestamp (ledger seconds) when PendingFeeConfig was proposed
    PendingFeeConfigProposedAt,
}
```

### New Types

A new `FeeConfigProposal` struct tracks the proposed config and proposal timestamp together:

```rust
#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeConfigProposal {
    pub config: FeeConfig,
    pub proposed_at: u64,
}
```

---

## New Functions

### `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

**Behavior:**

1. Require admin auth via `require_admin`.
2. Validate the proposed config using `Self::validate_fee_config`.
3. Retrieve the currently active `FeeConfig`.
4. Check if any individual fee field is greater than the current config:
   - If all are ≤ current (pure decrease or no change), immediately call `activate_fee_config` logic internally and return (emit both `fee_config_proposed` and `fee_config_updated` in the same transaction).
   - If any field is > current (increase or mixed), proceed to step 5.
5. Store the proposed config in `DataKey::PendingFeeConfig` and the current timestamp in `DataKey::PendingFeeConfigProposedAt`.
6. Emit `fee_config_proposed` event with `(admin, proposed_config, proposed_at)`.
7. Return success.

**Auth:** Current admin must sign.

**Errors:** `NotInitialized`, `InvalidInput` (validation failure), `PendingFeeConfigAlreadyExists`. (Admin auth is enforced by `require_admin`, which fails with `NotInitialized` when no admin is set and otherwise traps on a missing signature.)

**Emits:** `fee_config_proposed` (always); may also emit `fee_config_updated` if activation is immediate (decreases only).

**TTL:** Persistent storage keys extend to full 30-day TTL.

---

### `activate_fee_config() -> Result<(), ScoutAccessError>`

**Behavior:**

1. Require admin auth via `require_admin`.
2. Retrieve the pending proposal: `DataKey::PendingFeeConfig` and `DataKey::PendingFeeConfigProposedAt`.
   - If either is missing, return `NoPendingFeeConfig`.
3. Check that enough time has elapsed: `now - proposed_at >= FEE_CONFIG_PROPOSAL_DELAY_SECS`.
   - If not, return `FeeConfigProposalNotReady` (delay has not elapsed).
4. Retrieve the currently active config.
5. Move the pending config to `DataKey::FeeConfig`.
6. Clear `DataKey::PendingFeeConfig` and `DataKey::PendingFeeConfigProposedAt` from storage.
7. Emit `fee_config_updated` event with `(admin, old_config, new_config)`.
8. Return success.

**Auth:** Current admin must sign.

**Errors:** `NotInitialized`, `NoPendingFeeConfig`, `FeeConfigProposalNotReady`, `Overflow`.

**Emits:** `fee_config_updated` with `(admin, old_active_config, newly_activated_config)`.

---

## Events

### `fee_config_proposed` (New)

**Topics:** `(event_name, admin)`  
**Data:** `(proposed_config: FeeConfig, proposed_at: u64)`

Emitted by `propose_fee_config` when a fee increase is proposed (i.e., not immediately activated).

Indexers and scouts can use this event to build a timeline of what fees will be in effect and when.

**Example:** At ledger timestamp 2000, admin proposes elite_sub_stroops = 2.0 XLM (increase from 1.0). Event is emitted with `proposed_at = 2000` and `activated_at_or_later = 2000 + 604800` (7 days).

### `fee_config_updated` (Existing, Unchanged)

**Topics:** `(event_name, admin)`  
**Data:** `(old_config: FeeConfig, new_config: FeeConfig)`

Emitted whenever a fee config *takes effect* (becomes active).

- After `activate_fee_config`: old = previous active, new = newly activated.
- After `propose_fee_config` for decreases only: old = previous active, new = newly activated (same transaction).
- After `update_fee_config`: old = previous active, new = newly activated, with **zero advance notice** — see `fee_config_delay_bypassed` below.
- After `initialize` (to establish baseline).

### `fee_config_delay_bypassed` (New — #1055)

**Topics:** `(event_name, admin)`  
**Data:** `(old_config: FeeConfig, new_config: FeeConfig)` — same shape as `fee_config_updated`

Emitted **only** by `update_fee_config`, in the same transaction as (and immediately after) its own `fee_config_updated` emission. This is purely additive: `fee_config_updated`'s own topics and data are unchanged for every caller, so existing consumers that only know about `fee_config_updated` are unaffected.

**Why this event exists:** `update_fee_config` and `activate_fee_config` both emit an otherwise-identical `fee_config_updated` event, so before this event was added, an indexer or auditor reading the event stream alone could not tell whether a given fee change went through the 7-day advance-notice delay or bypassed it entirely via `update_fee_config` (see "Migration and Backwards Compatibility" below — `update_fee_config` remaining available and unrestricted is intentional, but it was previously *silently* unrestricted from an observability standpoint).

**How to interpret the event stream:**

| Same-transaction event(s) alongside `fee_config_updated` | What happened |
|---|---|
| `fee_config_delay_bypassed` | `update_fee_config` was called — the delay was bypassed entirely, no advance notice was given |
| `fee_config_proposed` (no `fee_config_delay_bypassed`) | `propose_fee_config` was called with an all-decreases config — immediately activated by design (decreases don't need advance notice; see "Fee Decreases: No Delay Required" above) |
| neither | `activate_fee_config` was called — the change went through the full 7-day delay as originally proposed |

---

## Business Logic Verification

### subscribe()

**Current:** Calls `Self::fee_config(&env)` → fetches from `DataKey::FeeConfig` → charges the active fee.

**After change:** No change needed. Will continue to fetch the active fee and never use a pending proposal.

### pay_to_contact()

**Current:** Calls `Self::fee_config(&env)` → charges active fee.

**After change:** No change needed. Will continue to fetch the active fee.

### log_trial_offer()

**Current:** No direct fee charged (trial offer is free for Elite scouts).

**After change:** No change needed. (Trial offer does not call `fee_config`.)

---

## Testing

### Unit Tests

1. **test_propose_fee_config_with_increase** — Propose a fee increase, verify:
   - Proposal is stored in `PendingFeeConfig` and `PendingFeeConfigProposedAt`.
   - `fee_config_proposed` event is emitted.
   - Active fee config remains unchanged.
   - `subscribe` still charges the old fee.

2. **test_propose_fee_config_with_decrease** — Propose a fee decrease, verify:
   - Fee config is immediately activated (no pending state).
   - Both `fee_config_proposed` and `fee_config_updated` events are emitted in the same transaction.
   - Active fee config is updated.
   - `subscribe` charges the new (lower) fee.

3. **test_activate_fee_config_before_delay** — Attempt to activate before the delay elapses:
   - Should return `FeeConfigProposalNotReady` error.

4. **test_activate_fee_config_after_delay** — Propose, advance ledger time by 7 days, then activate:
   - Pending config moves to active.
   - `fee_config_updated` event is emitted with `(old, new)`.
   - `subscribe` charges the new fee.

5. **test_activate_with_no_pending_proposal** — Attempt to activate when no proposal exists:
   - Should return `NoPendingFeeConfig` error.

6. **test_subscribe_during_proposal_window** — Subscribe between proposal and activation:
   - Should use the old (active) fee, never the pending one.

7. **test_pay_to_contact_during_proposal_window** — Contact during proposal window:
   - Should use the old (active) fee.

8. **test_mixed_increase_and_decrease** — Propose config where some fees are up, some down:
   - Proposal is treated as "increase" (at least one fee went up).
   - Requires the 7-day delay.

### Integration Tests

- Full flow: propose fee increase → subscribe with old fee → wait 7 days → activate → subscribe with new fee.
- Multi-config: two separate proposals, first activated, second pending, verify no confusion.
- Admin auth failures on both functions.

---

## CONTRACT_REFERENCE.md Entries

Two function entries are documented under `scout_access` in
[CONTRACT_REFERENCE.md](CONTRACT_REFERENCE.md#scout_access):

#### `propose_fee_config(fee_config: FeeConfig) -> Result<(), ScoutAccessError>`

Propose a new fee configuration. If all fees are ≤ current fees (decreases only), the config is immediately activated. Otherwise, it is stored as pending and requires `activate_fee_config` after a 7-day delay to take effect.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` · `InvalidInput` (validation failure) · `PendingFeeConfigAlreadyExists` |
| **Emits** | `fee_config_proposed` (always if no immediate activation); may also emit `fee_config_updated` for decreases |
| **Delay** | 7 days (604,800 seconds) for increases; none for decreases |

#### `activate_fee_config() -> Result<(), ScoutAccessError>`

Activate a pending fee configuration proposal after the 7-day delay has elapsed.

| | |
|---|---|
| **Auth** | Current admin must sign |
| **Errors** | `NotInitialized` · `NoPendingFeeConfig` · `FeeConfigProposalNotReady` (delay not yet elapsed) · `Overflow` |
| **Emits** | `fee_config_updated` with `(admin, old_config, new_config)` |

---

## Migration and Backwards Compatibility

### Option: Coexist (Chosen)

**Decision:** `update_fee_config` continues to exist and works as before (atomic, immediate). `propose_fee_config` + `activate_fee_config` are new, opt-in functions.

**Rationale:**
- Existing integrations and workflows that depend on immediate `update_fee_config` are not disrupted.
- Admins can migrate fee changes to the propose/activate flow at their own pace.
- Both paths coexist peacefully: if one path is used, the other's data (e.g., `PendingFeeConfig`) is not affected.

> [!NOTE]
> **`update_fee_config` is a known, auditable operational escape hatch.**
> Because `update_fee_config` remains callable by the admin at any time, it can always be used to bypass the "7 days of advance notice" guarantee that `propose_fee_config` / `activate_fee_config` exists to provide — this is an intentional trade-off (see "Rationale" above), not a bug, and it is not disabled or restricted by this design. As of #1055, this bypass is no longer *silent*: `update_fee_config` emits an additional `fee_config_delay_bypassed` event (see "Events" above) alongside its `fee_config_updated` event, so indexers, scout-facing dashboards, and auditors can always tell whether a given active `FeeConfig` change was given the full 7-day delay or not, purely from the on-chain event stream.

---

## Rollback and Edge Cases

### What if `activate_fee_config` is never called?

A pending proposal sits in `PendingFeeConfig` indefinitely. Scouts see the proposal via the `fee_config_proposed` event and know a change is pending, but the active fees never change. This is safe (conservative).

### What if a new proposal is submitted before the previous one is activated?

**Option: Reject overlapping proposals** (chosen for clarity)

Attempting to call `propose_fee_config` when a pending proposal already exists returns `PendingFeeConfigAlreadyExists` error.

Admin must either:
1. Call `activate_fee_config` to finalize the pending proposal, then propose a new one.
2. Manually clear the pending state (not exposed; requires contract upgrade).

This prevents confusion about which proposal will be active when.

> **Remaining item:** a proposal-*cancellation* path and its event are not
> implemented. Today a pending proposal can only be cleared by activating it
> or by a contract upgrade. This is the sole open gap for this feature and is
> tracked separately.

---

## Summary

1. **Delay Model:** Fixed 7-day constant.
2. **Business Logic:** Subscriptions and payments always use the currently active config.
3. **Fee Decreases:** Bypass the delay and activate immediately.
4. **Events:** New `fee_config_proposed` event for proposals; existing `fee_config_updated` for all activations; new `fee_config_delay_bypassed` event, additive to `fee_config_updated`, marking specifically the `update_fee_config` bypass path (#1055).
5. **Coexistence:** `update_fee_config` unchanged (still atomic, immediate, unrestricted) but its delay-bypass is no longer silent — it's now distinguishable from `activate_fee_config` in the event stream; `propose_fee_config` and `activate_fee_config` are new.
6. **Safety:** Pending proposals prevent scouts from being blindsided by fee increases.
