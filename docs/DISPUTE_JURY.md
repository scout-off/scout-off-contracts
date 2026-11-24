# Dispute Jury — Eligibility Rules

This document describes who may cast a vote on a jury-required milestone dispute
and what happens when a validator is revoked while a dispute is open.

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

## Revocation Vote Removal (both severities)

When `revoke_validator` is called — with **either** `RevocationSeverity::Routine`
or `RevocationSeverity::ForCause` — the contract's internal helper
`remove_dispute_votes_for_validator` is invoked immediately after the validator
record is deactivated.  The helper:

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

### Why both severities

Previously only `ForCause` revocations removed jury votes; a `Routine`
revocation (e.g. "validator left the academy") left the validator's votes
counting toward quorum and outcome even though they were no longer an active
validator and could not cast those votes now.  We now remove dispute votes for
**both** severities, consistent with the pending-attestation handling
(`invalidate_pending_votes_for_validator`), which already runs for both.

Two threats were weighed:

- **Stale votes counting (the bug).**  A revoked validator's vote keeps
  contributing to quorum and to the majority verdict.  This is a correctness
  problem: the tally no longer reflects the set of currently-eligible jurors,
  and a routine revocation could silently tip an open dispute.
- **Routine revocation as a vote-removal tool.**  If routine revocation removes
  votes, an admin could revoke a juror whose vote they dislike to strip that
  vote without triggering the for-cause cascade.

We judge the stale-vote threat to matter more.  Revocation is an admin-only,
accountable action that deactivates the validator entirely (they lose all
future voting power, not just this one vote), so it is a blunt instrument for
manipulating a single dispute.  Leaving stale votes in the tally, by contrast,
is a silent correctness defect that any routine revocation triggers.  Removing
votes uniformly also keeps the two severities' semantics aligned and avoids a
subtle behavioural difference that is easy to get wrong.

### Escalation is idempotent

`handle_inactive_re_revoke` may later escalate a `Routine` revocation to
`ForCause`.  Because the routine revocation already removed the validator's
dispute votes, the escalation's `remove_dispute_votes_for_validator` call finds
no `DisputeVote` records and is a no-op: it does not double-decrement
`DisputeVoteCount` or the tally counters, and does not panic on missing votes.

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

revoke_validator(Routine | ForCause)
  → remove_dispute_votes_for_validator(wallet)
      for each open dispute: undo vote tally if this validator voted

handle_inactive_re_revoke(Routine → ForCause)
  → remove_dispute_votes_for_validator(wallet)   // idempotent no-op if already removed

tally_dispute(...)
  → quorum met or deadline passed → resolve with majority verdict
```
