use soroban_sdk::contracterror;
use scoutchain_shared_types::AdminError;

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
}

impl AdminError for VerificationError {
    fn not_initialized() -> Self {
        VerificationError::NotInitialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env, String};

    const VALID_CID_V0: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";

    fn setup() -> (Env, crate::VerificationContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, crate::VerificationContract);
        let client = crate::VerificationContractClient::new(&env, &id);
        (env, client)
    }

    #[test]
    fn test_approve_milestone_description_at_boundary_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let validator = Address::generate(&env);
        client.initialize(&admin);
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"));

        let description_256 = String::from_str(&env, &"a".repeat(256));
        let evidence = String::from_str(&env, VALID_CID_V0);

        let result = client.try_approve_milestone(&validator, &1u64, &description_256, &evidence);
        assert!(result.is_ok(), "256-byte description should succeed");
    }

    #[test]
    fn test_approve_milestone_description_over_limit_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let validator = Address::generate(&env);
        client.initialize(&admin);
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"));

        let description_257 = String::from_str(&env, &"a".repeat(257));
        let evidence = String::from_str(&env, VALID_CID_V0);

        let result = client.try_approve_milestone(&validator, &1u64, &description_257, &evidence);
        assert_eq!(
            result,
            Err(Ok(VerificationError::InvalidInput)),
            "257-byte description should return InvalidInput"
        );
    }
}
