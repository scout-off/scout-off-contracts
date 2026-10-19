# Dispute Jury — Eligibility Rules

This document describes who may cast a vote on a jury-required milestone dispute
and what happens when a validator is revoked for cause while a dispute is open.

## Background

When a player calls `dispute_milestone` with an `impact_score` that meets or
exceeds the configured `JuryConfig.impact_threshold`, the dispute is routed to
the jury path (`jury_required = true`).  During the voting window any active,
eligible validator may call `cast_dispute_vote`.  Once the window closes (or
quorum is reached with a clear majority) anyone may call `tally_dispute` to
finalise the outcome.

## Eligibility Snapshot (`jury_eligibility_cutoff`)

At filing time, `dispute_milestone` records:

```
dispute.jury_eligibility_cutoff = env.ledger().timestamp()   // "now"
```

A validator **must** satisfy `validator.registered_at < jury_eligibility_cutoff`
to cast a vote.  Validators registered *after* the dispute was filed are rejected
with `NotEligibleJuror` (error code 44).

**Rationale**: without this snapshot, an admin (or a compromised admin key) could
register N new validators mid-vote and immediately control the outcome.

## Conflict-of-Interest Exclusions

Two categories of validators are excluded regardless of registration date:

| Rule | Check | Error returned |
|------|-------|----------------|
| Milestone approver | `milestone.validator == voter` | `ConflictOfInterest` (code 40) |
| Same-affiliation colleague | `validator.affiliation == dispute.approver_affiliation` | `ConflictOfInterest` (code 40) |

`approver_affiliation` is snapshotted from the approver's `Validator` record at
filing time and stored in the `MilestoneDispute` struct.  Changes to the
approver's record after filing do not affect an open dispute.

**Rationale**: validators from the same academy or organisation share an
institutional interest in protecting their colleague's approval.  Excluding
them prevents bloc-voting within a single affiliation.

## For-Cause Revocation Vote Removal

When `revoke_validator` is called with `RevocationSeverity::ForCause`, the
contract's internal helper `remove_dispute_votes_for_validator` is invoked
immediately after the validator record is deactivated.  The helper:

1. Iterates the `OpenDisputeIndex` (all unresolved disputes).
2. For each dispute, checks whether a `DisputeVote` record exists for the
   revoked wallet.
3. If found, decrements the appropriate tally counter
   (`votes_for` or `votes_against`) on the `MilestoneDispute` record and
   removes the `DisputeVote` storage entry.
4. Also decrements the `DisputeVoteCount` counter for that dispute.

`ActiveDisputesCount` and `OpenDisputeIndex` are kept consistent: the dispute
itself is not removed (it remains open for further voting), only the revoked
validator's individual vote contribution is undone.

**Rationale**: a validator revoked for serious misconduct must not retain
influence over ongoing jury decisions.  Routine (`Routine`) revocations do not
trigger vote removal — only `ForCause` does.

## Error Reference

| Code | Variant | When raised |
|------|---------|-------------|
| 40 | `ConflictOfInterest` | Voter is the milestone approver, or shares `affiliation` with the approver |
| 41 | `AlreadyVoted` | Validator has already cast a vote on this dispute |
| 42 | `VotingWindowOpen` | `tally_dispute` called before deadline with a tied vote at/above quorum |
| 43 | `QuorumNotReached` | `tally_dispute` called before deadline and quorum not yet met |
| 44 | `NotEligibleJuror` | Validator registered after `jury_eligibility_cutoff` |

## Summary Flow

```
dispute_milestone(...)
  → snapshot jury_eligibility_cutoff = now
  → snapshot approver_affiliation from Validator record

cast_dispute_vote(validator, ...)
  → validator.registered_at < jury_eligibility_cutoff ?  else → NotEligibleJuror
  → milestone.validator != voter ?                        else → ConflictOfInterest
  → validator.affiliation != approver_affiliation ?       else → ConflictOfInterest
  → no prior vote ?                                       else → AlreadyVoted
  → record vote, update tally

revoke_validator(ForCause)
  → remove_dispute_votes_for_validator(wallet)
      for each open dispute: undo vote tally if this validator voted

tally_dispute(...)
  → quorum met or deadline passed → resolve with majority verdict
```
