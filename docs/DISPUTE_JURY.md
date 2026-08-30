# Dispute & Jury System

> **Status: Partially implemented.** The progress contract exposes a
> `reset_player_level` admin function for dispute resolution. A dedicated
> `resolve_dispute` entrypoint and jury/k-of-n gating do **not yet exist** in
> the contract source. This document describes what is currently in the code and
> flags the missing guards as bugs. See the linked issues for tracking.

---

## Current Implementation

### `progress::reset_player_level`

The only dispute-resolution mechanism in the codebase today is
`reset_player_level` in the progress contract:

```
progress::reset_player_level(env, player_id, target_level) → Result<(), ProgressError>
```

| Property | Detail |
|----------|--------|
| Caller | Admin only (enforced by `require_admin`) |
| Guard | Contract must not be paused |
| Effect | Sets the player's level to `target_level`; records a history entry with the old → new level; emits `player_level_reset` event |
| Cross-contract | Syncs the new level to the registration contract if wired |

**There is no jury gating, no `DisputeRequiresJury` guard, and no
`resolve_dispute` function in `contracts/verification/src/lib.rs` or anywhere
else in the workspace.**

---

## Bug Report: Missing Jury Guard on Admin Dispute Resolution

> **🐛 BUG — relates to issue #1263**

### Description

`docs/DISPUTE_JURY.md` (this file) was written under the assumption that admin
dispute resolution is blocked for jury-required disputes and returns a
`DisputeRequiresJury` error. Code inspection shows:

- `contracts/verification/src/errors.rs` has **no** `DisputeRequiresJury`
  variant. The enum tops out at `ValidatorCapReached = 15`.
- There is **no** `resolve_dispute` function in
  `contracts/verification/src/lib.rs`.
- The `reset_player_level` function in `contracts/progress/src/lib.rs` is the
  only admin path for changing a player's level after a dispute. It performs
  **no jury check** — any admin can reset any player's level at any time as long
  as the contract is not paused.

### Impact

Because there is no jury gate on the dispute-resolution path:

1. Admin can resolve a dispute unilaterally that should require a jury quorum.
2. The `ActiveDisputesCount` double-decrement path described in the related
   Medium issue is reachable with no additional preconditions.
3. The claim in this file (and any previous version of it) that
   `DisputeRequiresJury` is returned is incorrect and misleading.

### Required Fix

Either:

**Option A** — Add a `DisputeRequiresJury = 16` variant to `VerificationError`
and implement a `resolve_dispute` function in `verification/src/lib.rs` that:
- Checks whether the dispute has a jury-required flag set.
- Returns `VerificationError::DisputeRequiresJury` (code 16) if the jury quorum
  has not been reached.
- Calls `progress::reset_player_level` cross-contract only after quorum is met.

**Option B** — Explicitly document that the jury feature is not yet implemented
and track it as a separate feature issue, removing any documentation that claims
the guard exists until it does.

---

## Error Reference (as of current source)

### `VerificationError` variants relevant to disputes

| Code | Variant | Notes |
|------|---------|-------|
| 4 | `Unauthorized` | Returned for any admin-level call made by a non-admin. Currently the closest error to a "blocked" dispute resolution. |
| — | `DisputeRequiresJury` | **Does not exist.** See bug report above. |

### `ProgressError` variants relevant to disputes

| Code | Variant | Notes |
|------|---------|-------|
| 3 | `ContractPaused` | `reset_player_level` returns this if the circuit breaker is active. |
| 4 | `Unauthorized` | `reset_player_level` returns this if caller is not admin. |

---

## Planned Jury Flow (not yet implemented)

When the jury feature is shipped the expected flow is:

```
Admin calls resolve_dispute(dispute_id)
    │
    ▼
Is dispute flagged as jury_required?
    │ YES
    ▼
Has jury quorum been reached?
    │ NO  → return VerificationError::DisputeRequiresJury (code 16)
    │ YES → proceed
    ▼
Cross-contract call: progress::reset_player_level(player_id, target_level)
    │
    ▼
Emit dispute_resolved event
```

Until this flow is implemented, `reset_player_level` remains an unrestricted
admin back-door that bypasses any future jury logic.

---

## Related Issues

- **#1263** — This doc: `DisputeRequiresJury` does not exist in `VerificationError`
- Medium issue — `ActiveDisputesCount` double-decrement (reachable because the
  jury guard is missing)
