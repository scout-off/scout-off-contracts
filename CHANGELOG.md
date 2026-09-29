# Changelog

This file records notable versioned changes to the ScoutOff contracts. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the repository versioning policy lives in [docs/VERSIONING.md](docs/VERSIONING.md).

## Entry conventions

Future entries should use the following structure:

- Version: `vMAJOR.MINOR.PATCH`
- Release date: `YYYY-MM-DD`
- Contracts affected: the contract or contracts changed by the release
- Summary: a short description of the externally observable change
- Classification: `Breaking (MAJOR)` or `Non-breaking (MINOR)`

Entries must be kept in reverse chronological order. Any pull request that requires a MINOR or MAJOR version bump must add or update the corresponding changelog entry.

The initial v0.1.0 entry below retains the year-only date because no exact historical release date is available. Adoption of this changelog does not require retroactive entries for earlier unversioned changes.

## Unreleased

Use the structure below for upcoming MINOR or MAJOR contract changes:

- Version: `vX.Y.Z`
- Release date: `YYYY-MM-DD`
- Contracts affected: `progress`, `registration`, `scout_access`, `verification` (or a subset)
- Summary: a concise description of the externally observable change
- Classification: `Breaking (MAJOR)` or `Non-breaking (MINOR)`

> **Breaking-change classification rules:** See [docs/VERSIONING.md — What Constitutes a Breaking Change](VERSIONING.md#what-constitutes-a-breaking-change) for the full criteria (storage layout changes, function signature changes, error code renumbering, event schema changes, cross-contract interface changes).

- Version: `v0.4.0`
- Release date: `2026-08-19`
- Contracts affected: `verification`
- Summary: Implemented dispute-jury escalation for high-impact milestone disputes (issue #1036). `MilestoneDispute` now records `impact_score`, `jury_required`, `quorum`, `votes_for`, `votes_against`, and `voting_deadline`; admin-only resolution remains for low-impact disputes while `set_jury_config`, `cast_dispute_vote`, and `tally_dispute` add the jury flow and the new event/state schema. Existing `MilestoneDispute` rows written before the upgrade require migration before the new WASM can decode them.
- Classification: `Breaking (MAJOR)`

- Version: `v0.3.0 (scout_access)`
- Release date: `2026-08-18`
- Contracts affected: `scout_access`
- Summary: Added escrow-backed trial offers. `log_trial_offer` now captures `trial_offer_escrow_stroops`, `expire_trial_offers(limit)` sweeps stale entries after `trial_offer_expiry_secs`, and `admin_refund_trial_escrow` provides a direct recovery path for an identified stuck escrow. The new runtime behavior is additive and remains bounded by the expiry window cap.
- Classification: `Non-breaking (MINOR)`

- Version: `v1.1.0`
- Release date: `2026-08-20`
- Contracts affected: `progress`, `registration`, `scout_access`, `verification`
- Summary: Added unauthenticated scalar peer-address getters (issue #1116) for six of the platform's eight cross-contract wiring links (the full set is enumerated in `docs/WIRING_REGISTRY_DESIGN.md`; the two `*.set_registration_contract` links on `verification` and `scout_access` remain reachable only via `get_wiring_state()`). Each getter returns `None` before configuration and the stored address afterward; existing aggregate `get_wiring_state()` APIs remain unchanged.
- Classification: `Non-breaking (MINOR)`

- Version: `v1.0.1`
- Release date: `2026-08-20`
- Contracts affected: `shared-tests` (removed — no deployed contract affected)
- Summary: Removed the orphaned `contracts/shared-tests` phantom crate (issue #1117). The directory contained only a `src/lib.rs` with no `Cargo.toml`, was never registered as a workspace member, and its four `#[test]` functions had empty placeholder bodies that asserted nothing. Real, workspace-compiled coverage for the same admin-transfer properties already exists in `contracts/{registration,verification,progress,scout_access}/tests/admin_transfer_properties.rs` (documented in `docs/ADMIN_TRANSFER_VERIFICATION.md`). The dead crate was originally deleted in #1070 (PR #1074, commit `9feba1a`); this entry formally closes the tracking issue #1117 and confirms `cargo test --workspace` and `cargo build --workspace` remain unaffected (the crate was never part of the workspace).
- Classification: `Non-breaking (PATCH)`

- Version: `v1.0.0`
- Release date: `2026-08-19`
- Contracts affected: `verification`
- Summary: Two major breaking changes. (1) Implemented the validator revocation cascade re-review system (issue #1039): `revoke_validator` now accepts an explicit `RevocationSeverity` enum (`Routine | ForCause`) and an optional reason string instead of inferring severity from a magic string. For `ForCause` revocations, the contract iterates the validator's `ValidatorMilestones` history and flags every referenced milestone as `MilestonePendingReReview` in a bounded, resumable sweep (50 per call); `continue_revocation_cascade` picks up where the initial call left off. New public entrypoints: `is_milestone_flagged`, `rereview_milestone` (active validator clears a flag after independent confirmation), `get_revocation_record`, `continue_revocation_cascade`. New error codes: `NotEligibleToReReview` (32), `MilestoneNotFlagged` (33). New events: `milestone_flagged`, `milestone_flag_cleared`, `revocation_cascade_complete`, `revocation_cascade_continued`. New types: `RevocationSeverity`, `RevocationRecord`. New SQL migration `007_milestone_flags.sql` adds `milestone_flags` and `revocation_records` indexer tables. (2) Implemented the dispute-jury escalation system (issue #1036): `MilestoneDispute` struct now carries six new fields for jury-based dispute resolution—`impact_score: u32`, `jury_required: bool`, `quorum: u32`, `voting_deadline: u64`, `votes_for: u32`, `votes_against: u32`. These fields enable on-chain voting by active validators when a milestone is disputed with `jury_required = true`. The change breaks XDR encoding for existing disputes on-chain. New SQL migration `006_dispute_jury.sql` adds indexer tables for jury voting. `batch_revoke_validators` updated with the same new signature. `scripts/check-abi-diff.sh` classifies parameter-list changes as MAJOR.
- Classification: `Breaking (MAJOR)`

> **Migration guide:** Any caller of `revoke_validator(wallet, reason)` or `batch_revoke_validators(wallets, reason)` must add an explicit `severity` parameter (second positional argument). For callers that previously passed `reason = Some("Routine")` to mean a routine deactivation, change to `severity = RevocationSeverity::Routine`. For all other reasons, use `RevocationSeverity::ForCause`. The behavior for `ForCause` is unchanged in terms of validator deactivation; the new cascade flagging is additive. For dispute-jury: existing `MilestoneDispute` entries written by v0.3.0 cannot be decoded by the v1.0.0 ABI. Operators must resolve or acknowledge all open disputes on the old contract **before** upgrading. Run `migrations/007_milestone_flags.sql` and `migrations/006_dispute_jury.sql` against the backend database before deploying this version.

- Version: `v0.4.0`
- Release date: `2026-08-19`
- Contracts affected: `verification`
- Summary: Implemented dispute-jury escalation for high-impact milestone disputes (issue #1036). `MilestoneDispute` now records `impact_score`, `jury_required`, `quorum`, `votes_for`, `votes_against`, and `voting_deadline`; admin-only resolution remains for low-impact disputes while `set_jury_config`, `cast_dispute_vote`, and `tally_dispute` add the jury flow and the new event/state schema. Existing `MilestoneDispute` rows written before the upgrade require migration before the new WASM can decode them.
- Classification: `Breaking (MAJOR)`

- Version: `v0.3.1 (scout_access)`
- Release date: `2026-08-18`
- Contracts affected: `scout_access`
- Summary: Implements the `EvidenceAccessGrant` confidential-evidence authorization system specified in [docs/EVIDENCE_PRIVACY.md](docs/EVIDENCE_PRIVACY.md) (#1040). `pay_to_contact` and `batch_contact_players` now atomically write an `EvidenceAccessGrant(player_id, scout)` — `{granted_at, tier_at_grant, revoked, revoked_at}` — and emit `evidence_access_granted` on every successful contact, never on a rejected one. New query API: `has_evidence_access(player_id, scout) -> bool`, `get_evidence_access_grant(player_id, scout) -> Option<EvidenceAccessGrant>`, and paginated `get_player_access_grants(player_id, offset, limit) -> Vec<EvidenceAccessGrant>` (capped at 50/page, backed by a fixed-size sharded index so cost stays flat regardless of a player's total historical grant count — proven at 1,000+ grants). New admin-gated `admin_revoke_evidence_access(player_id, scout)` marks a grant revoked (never deletes it) and emits `evidence_access_revoked`; grants are append-only facts about a past, successful contact and are **not** auto-revoked by subscription downgrade or expiry — see EVIDENCE_PRIVACY.md for the full rationale and the caveat that revocation only gates future off-chain key-wrap requests. One error code appended: `GrantNotFound` (38; the CHANGELOG originally mis-described this as 30, which is taken by `SubscriptionAlreadyExists` — 38 is the actual next free append-only slot after `TrialEscrowNotOutstanding` = 37). `migrations/004_evidence_access_grants.sql` adds the off-chain mirror table; `scripts/reconcile-indexer.js` reconciles it.
- Classification: `Non-breaking (MINOR)`

- Version: `v0.3.0`
- Release date: `2026-08-18`
- Contracts affected: `progress`, `registration`, `scout_access`, `verification`
- Summary: Completed the cross-contract wiring observability rollout (issue #1041). Added `get_wiring_state()` to `registration`, `verification`, and `scout_access` (joining the existing `progress` implementation), sharing a common `WiringLink { address, epoch }` pattern from a new `scoutchain-shared-types` helper (`read_wiring_link`/`write_wiring_link`). Every `set_*_contract`/`update_*_contract` setter across all four contracts now bumps a per-link re-wiring epoch and emits a new `wiring_updated` event on every successful call, in addition to any pre-existing event. `verification`'s two first-call-only setters (`set_progress_contract`, `set_registration_contract`) keep their `AlreadyConfigured` guard unchanged — `update_progress_contract`/`update_registration_contract` remain the deprecated-but-functional re-wiring path — every other setter across all four contracts remains freely re-settable, as before. `scripts/verify-cross-contract-wiring.sh` is rewritten to call `get_wiring_state()` on all four contracts (previously only `progress`), group the platform's eight peer-address pointers by target contract, and classify each group as `FULLY_WIRED` / `NEVER_CONFIGURED` / `PARTIAL` — explicitly detecting a partially-applied re-wiring (some dependents updated, others stale) as a distinct failure mode; a new `--repair` flag prints the exact corrective `stellar contract invoke` command(s). `scripts/full-readiness-check.sh` mirrors the same classification. `scripts/initialize.sh` now wires the three previously-missing links (`verification` → `registration`, `progress` → `scout_access`, `scout_access` → `registration`) and gates its own success on a post-wiring `verify-cross-contract-wiring.sh` run rather than assuming success from the absence of individual invoke errors.
- Classification: `Non-breaking (MINOR)` — every new storage key is additive (verified via `scripts/check-storage-layout-compat.sh`), and no existing function's signature, error codes, or behavior changed for a caller that doesn't use the new getters/events.

- Version: `v0.2.0 (verification)`
- Release date: `2026-07-29`
- Contracts affected: `verification`
- Summary: Added `attest_milestone`, an on-chain k-of-n threshold consensus scheme for milestone approval. `attest_milestone(validator_wallet, player_id, description, evidence_hash)` records one independent, asynchronous vote per call; once `threshold` distinct, currently-active validators have voted for the same `(player_id, evidence_hash)` claim within a configurable voting window, the milestone commits and `progress.advance_level` is cross-called exactly once. Also added `set_milestone_threshold`/`get_milestone_threshold`, `set_voting_window_secs`/`get_voting_window_secs`, `get_pending_claim`, `has_attested`, and `is_attestation_window_expired`; three error codes appended (`DuplicateAttestation` 26, `TooManyPendingVotes` 27, `ThresholdModeRequiresAttestation` 28); `revoke_validator`/`batch_revoke_validators` now retroactively strip a revoked validator's still-open vote from any pending claim's tally. `approve_milestone`'s signature and default behavior (`threshold = 1`) are unchanged for existing callers; it is only gated (`ThresholdModeRequiresAttestation`) once an operator opts in via `set_milestone_threshold(n >= 2)`. A follow-up audit closed a gap where `submit_attested_milestone` (the off-chain ed25519-relay commit path) did not check the same threshold gate and remained a single-signature bypass of k-of-n mode; also fixed `has_attested` returning a stale `true` for votes past an unrolled-over expired window, and a `MAX_PENDING_VOTES_PER_VALIDATOR` bookkeeping bug that double-counted a validator's own claim when their revote was what triggered that claim's lazy round-bump.
- Classification: `Non-breaking (MINOR)`

- Version: `v0.2.0`
- Release date: `2026-07-28`
- Contracts affected: `scout_access`
- Summary: `batch_contact_players` now returns `ProContactLimitReached` (20) instead of `ContactQuotaExceeded` (18) when the Pro-tier monthly contact limit is exceeded. Error code 18 is reserved/deprecated. `check_pro_contact_quota_with_count` unified with `pay_to_contact`'s inline quota check on the same error code.
- Classification: `Breaking (MAJOR)`

> **Migration guide:** Clients that previously matched `ContactQuotaExceeded` (18) from `batch_contact_players` must update to `ProContactLimitReached` (20). Both `pay_to_contact` and `batch_contact_players` now return the same error code for equivalent quota-exceeded states.

## v0.1.0 - 2025

- Version: `v0.1.0`
- Release date: `2025`
- Contracts affected: `progress`, `registration`, `scout_access`, `verification`
- Summary: Initial release — all four contracts with full test coverage
  - Baseline includes milestone disputes, batch contact operations, escrow-backed
    trial offers, and Pro-tier contact quotas; these were part of v0.1.0
    rather than later unversioned additions.
- Classification: `Non-breaking (initial release baseline)`

This entry is treated as the baseline for the initial public release rather than a change from an earlier public version.
