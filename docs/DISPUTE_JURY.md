# Dispute Jury Escalation Design

> **Status: Implemented** — shipped in `feature/dispute-jury-escalation-1036` (issue #1036).
> See `docs/CONTRACT_REFERENCE.md` for the complete function reference and
> `contracts/verification/src/lib.rs` for the canonical implementation.

This document specifies the multi-validator jury mechanism for high-impact milestone
disputes in the verification contract.

## Overview

When a milestone approval is challenged, the contract records a `MilestoneDispute`.
Low-impact disputes use the existing **admin-only** `resolve_dispute` path.
High-impact disputes (at or above a configurable threshold) require a **jury vote**
from independent validators before the dispute can be finalized.

Player progress is **not** rolled back automatically when a dispute is upheld; resolution
records the outcome on-chain for auditability and off-chain follow-up, matching the
existing admin path behavior.

## Impact threshold

| Path | Condition | Resolution |
|------|-----------|------------|
| Admin-only | `impact_score < jury_config.impact_threshold` | Admin calls `resolve_dispute` |
| Jury | `impact_score >= jury_config.impact_threshold` | Validators vote; `tally_dispute` finalizes |

Default threshold: **100** (admin-configurable via `set_jury_config`).

## Eligibility to vote

A validator may call `cast_dispute_vote` only when **all** of the following hold:

1. Wallet is a registered **active** validator.
2. Validator is **not** the original approver of the disputed milestone (conflict of interest).
3. Validator has **not** already voted on this dispute.
4. Dispute is **jury-required**, **unresolved**, and the voting window is **still open**.

The dispute filer is not restricted unless they are also the original approver (rule 2).

## Quorum and voting window

| Parameter | Default | Description |
|-----------|---------|-------------|
| `quorum` | 3 | Minimum votes before a jury outcome can be recorded |
| `voting_window_secs` | 604800 (7 days) | Seconds after filing when voting closes |

Configurable via `set_jury_config` (admin only).

A dispute snapshots its quorum and voting deadline when it is filed, so later
configuration changes cannot alter an in-progress vote.

## Tie-breaking

When `tally_dispute` runs:

- **Majority for** (`votes_for > votes_against`): dispute **upheld** (`upheld = true`).
- **Majority against** (`votes_against > votes_for`): dispute **rejected** (`upheld = false`).
- **Tie** (`votes_for == votes_against`): dispute **rejected** (`upheld = false`); the original milestone stands.

## When tally is allowed

`tally_dispute` succeeds when the dispute is jury-required and unresolved, and either:

1. **Early close**: total votes ≥ quorum **and** `votes_for ≠ votes_against` (clear majority), or
2. **Deadline passed**: current time ≥ `voting_deadline` (majority rules apply; if total votes < quorum, outcome is **not upheld**).

Admin `resolve_dispute` is **blocked** for jury-required disputes (`DisputeRequiresJury`).

## Events

| Event | When |
|-------|------|
| `milestone_disputed` | Dispute filed |
| `dispute_vote_cast` | Validator casts a vote |
| `dispute_tallied` | Jury outcome finalized |
| `dispute_resolved` | Admin resolves a low-impact dispute |

## Storage

- `MilestoneDispute(player_id, milestone_index)` — dispute record and vote counters
- `DisputeVote(player_id, milestone_index, validator)` — individual vote (audit trail)
- `JuryConfig` — instance-level threshold, quorum, and window
