use scoutchain_shared_types::AdminError;
use soroban_sdk::contracterror;

/// Append-only: do not renumber existing variants. See docs/CONTRIBUTING.md.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum VerificationError {
    // ── Initialization & lifecycle ──
    /// `initialize` called more than once.
    AlreadyInitialized = 1,
    /// Operation before `initialize`.
    NotInitialized = 2,
    /// Circuit breaker is active.
    ContractPaused = 3,
    /// `set_progress_contract` called twice.
    AlreadyConfigured = 11,

    // ── Authorization ──
    /// Wrong account for a privileged operation.
    Unauthorized = 4,

    // ── Validators ──
    /// Wallet not in validator registry.
    ValidatorNotFound = 5,
    /// Validator has been revoked.
    ValidatorInactive = 6,
    /// Wallet already registered as validator.
    ValidatorAlreadyRegistered = 7,
    /// 100-validator limit reached; contract upgrade required to raise the cap.
    ValidatorCapReached = 15,

    // ── Milestones & evidence ──
    /// Invalid `player_id`.
    PlayerNotFound = 8,
    /// Index out of range.
    MilestoneNotFound = 14,
    /// Evidence hash has already been used in a prior `approve_milestone` call.
    DuplicateEvidence = 16,
    /// Validator has already approved 5 milestones for this player.
    MilestoneLimitExceeded = 17,
    /// Dispute was already resolved and cannot be resolved again.
    DisputeAlreadyResolved = 18,

    // ── Input validation ──
    /// Bad evidence hash or credentials too long.
    InvalidInput = 9,
    /// Revocation reason exceeds 128 bytes.
    ReasonTooLong = 10,

    // ── Cross-contract & arithmetic ──
    /// Cross-contract `advance_level` failed.
    ProgressCallFailed = 12,
    /// Milestone counter overflowed.
    Overflow = 13,

    // ── Admin transfer ──
    /// `accept_admin` called before an admin transfer was proposed.
    PendingAdminNotSet = 19,

    // ── Function-scoped pausing ──
    /// The approve_milestone function is paused independently of whole-contract pause.
    ApproveMilestonePaused = 20,

    // ── Specialization ──
    /// Validator is not tagged for the requested milestone category.
    /// Only raised when a `milestone_category` is supplied to `approve_milestone`
    /// and the validator's `specializations` list does not contain that category.
    SpecializationMismatch = 21,

    // ── Off-chain attestation (issue #703) ──
    /// ed25519 signature over the attestation payload failed verification,
    /// or the payload's contract/network binding does not match this instance.
    InvalidAttestation = 22,
    /// No attestation public key has been registered for this validator.
    AttestationKeyNotFound = 23,
    /// Attestation nonce is not strictly greater than the last accepted nonce.
    InvalidNonce = 24,
    /// Validator registration attempted before the cooldown window elapsed.
    RegistrationCooldown = 25,

    // ── Registration cross-contract ──
    /// Cross-contract call to the registration contract failed.
    RegistrationCallFailed = 29,

    // ── k-of-n threshold milestone attestation ──
    /// The same active validator has already attested to this exact
    /// (player_id, evidence_hash) claim within its current voting round.
    /// Distinct from a successful first-time `AttestationStatus::Pending`.
    DuplicateAttestation = 26,
    /// This validator already has `MAX_PENDING_VOTES_PER_VALIDATOR`
    /// concurrent open (sub-threshold, unexpired) attestation votes
    /// outstanding; wait for one to resolve (commit or expire) before
    /// opening another.
    TooManyPendingVotes = 27,
    /// `approve_milestone` or `submit_attested_milestone` was called while
    /// `get_milestone_threshold() > 1`. Once an operator opts into k-of-n
    /// mode, all milestone commitments must go through `attest_milestone` —
    /// there is no single-signature bypass, on-chain or off-chain-relayed,
    /// once threshold >= 2 is configured.
    ThresholdModeRequiresAttestation = 28,

    // ── Migration ──
    /// Migration window is not currently active on this contract.
    /// Call `open_migration_window` (admin-only) before seeding state.
    MigrationNotActive = 30,
    /// A `Milestone` already exists at `(player_id, milestone_index)` with
    /// different content. Identical replays are no-ops; conflicting replays
    /// are rejected to prevent silent overwriting of committed milestones.
    MilestoneAlreadyExists = 31,
    /// A `MilestoneDispute` already exists at `(player_id, milestone_index)`
    /// with different content. Identical replays are no-ops.
    DisputeAlreadyExists = 32,
    /// `restore_validator_record` targeted a validator entry whose archival
    /// grace period has fully elapsed (evicted, not merely marked inactive) and
    /// is unrecoverable.
    ValidatorRecordEvicted = 33,
    /// `restore_milestone_record` targeted a milestone entry that has been
    /// fully evicted and is unrecoverable.
    MilestoneRecordEvicted = 34,

    // ── Revocation cascade re-review (issue #1039) ──
    /// `rereview_milestone` called by a wallet that is not a currently-active
    /// validator. Only active (non-revoked) validators may clear a pending
    /// re-review flag.
    NotEligibleToReReview = 35,
    /// `rereview_milestone` called on a milestone that is not currently flagged
    /// as pending re-review. The flag either never existed or was already
    /// cleared by a prior `rereview_milestone` call.
    MilestoneNotFlagged = 36,

    // ── Jury escalation system (issue #1036) ──
    /// `resolve_dispute` called on a dispute that requires jury resolution.
    /// Use `tally_dispute` to finalize jury-required disputes.
    DisputeRequiresJury = 37,
    /// `cast_dispute_vote` or `tally_dispute` called on a dispute that was
    /// not routed to the jury path (jury_required == false).
    NotJuryDispute = 38,
    /// `cast_dispute_vote` called after the voting window has closed.
    VotingWindowClosed = 39,
    /// `cast_dispute_vote` called by the validator who originally approved
    /// the disputed milestone (conflict of interest).
    ConflictOfInterest = 40,
    /// `cast_dispute_vote` called by a validator who has already voted on
    /// this specific dispute.
    AlreadyVoted = 41,
    /// `tally_dispute` called before the voting window closes and when the
    /// vote count is tied at or above quorum (cannot resolve early on a tie).
    VotingWindowOpen = 42,
    /// `tally_dispute` called before the voting window closes and the
    /// required quorum of votes has not yet been reached.
    QuorumNotReached = 43,
}

impl AdminError for VerificationError {
    fn not_initialized() -> Self {
        VerificationError::NotInitialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

    const VALID_CID_V0: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";

    fn setup() -> (Env, crate::VerificationContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(crate::VerificationContract, ());
        let client = crate::VerificationContractClient::new(&env, &id);
        (env, client)
    }

    #[test]
    fn test_approve_milestone_description_at_boundary_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let validator = Address::generate(&env);
        client.initialize(&admin);
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let description_256 = String::from_str(&env, &"a".repeat(256));
        let evidence = String::from_str(&env, VALID_CID_V0);

        let result =
            client.try_approve_milestone(&validator, &1u64, &description_256, &evidence, &None);
        assert!(result.is_ok(), "256-byte description should succeed");
    }

    #[test]
    fn test_approve_milestone_description_over_limit_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let validator = Address::generate(&env);
        client.initialize(&admin);
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let description_257 = String::from_str(&env, &"a".repeat(257));
        let evidence = String::from_str(&env, VALID_CID_V0);

        let result =
            client.try_approve_milestone(&validator, &1u64, &description_257, &evidence, &None);
        assert_eq!(
            result,
            Err(Ok(VerificationError::InvalidInput)),
            "257-byte description should return InvalidInput"
        );
    }
}
