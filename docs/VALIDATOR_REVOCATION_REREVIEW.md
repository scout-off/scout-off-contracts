# Validator Revocation Re-review Cascade

> **Status: Implemented** — shipped in PR #1039. See `CHANGELOG.md` for the
> MAJOR version entry and `docs/CONTRACT_REFERENCE.md` for the full function
> reference.

## Purpose

Revoking a validator stops future approvals. A revocation for misconduct also
marks that validator's prior approvals as pending re-review so scouts and
indexers can identify affected milestones without correlating every validator
record manually.

## Revocation severity

The administrator supplies a `RevocationSeverity` and reason when calling
`revoke_validator`.

| Severity | Effect on prior milestones |
|---|---|
| `Routine` | Validator is deactivated; no prior milestone flags change. |
| `ForCause` | Validator is deactivated and all indexed prior approvals are flagged. |

The severity, reason, and revocation time are retained in a `RevocationRecord`.

## Cascade and re-review

Every approval is indexed in the approving validator's history. On a for-cause
revocation, the contract iterates that history and sets a
`MilestonePendingReReview` flag for each referenced milestone. This does not
roll back player levels or delete historical milestones.

Off-chain consumers query `is_milestone_flagged(player_id, milestone_index)` to
surface the warning. An active validator may call `rereview_milestone` to clear
one pending flag after independently confirming the underlying achievement.
Both flagging and clearing emit audit events.

## Bounded, resumable sweep

A prolific validator may have approved hundreds of milestones. The cascade sweep
is bounded at `CASCADE_LIMIT = 50` flags per call. If the validator's history
exceeds this limit, `revoke_validator` flags the first 50 and stores a cursor
under `DataKey::RevocationCascadeCursor(wallet)`. Call
`continue_revocation_cascade(wallet)` (admin only) one or more times until the
`revocation_cascade_complete` event is emitted, indicating the full history has
been flagged.

This mirrors the `expire_trial_offers` bounded-sweep pattern in
`contracts/scout_access/src/lib.rs`.

## On-chain functions (verification contract)

| Function | Auth | Description |
|---|---|---|
| `revoke_validator(wallet, severity, reason)` | Admin | Deactivate a validator with explicit severity; starts cascade for ForCause. |
| `continue_revocation_cascade(wallet)` | Admin | Resume an in-progress cascade sweep. |
| `is_milestone_flagged(player_id, milestone_index)` | Public | Returns `true` if the milestone is pending re-review. |
| `rereview_milestone(reviewer, player_id, milestone_index)` | Active validator | Clear a pending flag after independently confirming the achievement. |
| `get_revocation_record(wallet)` | Public | Return the stored `RevocationRecord` for a revoked validator. |

## Events

| Event | When |
|---|---|
| `milestone_flagged` | Each milestone flagged during a for-cause cascade. |
| `milestone_flag_cleared` | A reviewer calls `rereview_milestone` successfully. |
| `revocation_cascade_complete` | The full ValidatorMilestones history has been flagged. |
| `revocation_cascade_continued` | CASCADE_LIMIT hit; cursor stored for continuation. |

## Off-chain schema

`migrations/007_milestone_flags.sql` creates:

- `milestone_flags` — one row per currently-flagged `(player_id, milestone_index)`.
- `revocation_records` — one row per revoked validator, mirroring on-chain `RevocationRecord`.

Reconcile via `scripts/reconcile-indexer.js` (see `docs/INDEXER.md`).

## Non-destructive guarantee

Flagging a milestone as `MilestonePendingReReview`:

- Does **not** roll back the player's progress level.
- Does **not** delete or modify the milestone record.
- Is purely an advisory signal for scouts, indexers, and dispute processes.

Disputes (`dispute_milestone` / `resolve_dispute`) and pending-re-review flags
are distinct, non-overlapping mechanisms. Do not conflate them.
