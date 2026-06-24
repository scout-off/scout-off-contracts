#![no_std]
// IMPORTANT: Cross-contract wiring required after deployment
//
// `approve_milestone` calls `advance_level` on the progress contract to update
// a player's progress level atomically. This link is NOT automatic — after
// deploying both contracts you MUST run:
//
//   stellar contract invoke --id $VERIFICATION_CONTRACT_ID \
//     -- set_progress_contract \
//     --progress_contract $PROGRESS_CONTRACT_ID
//
// The easiest way is to run `./scripts/initialize.sh` which does this for you.
// Without this step, milestones are recorded but player levels will NOT advance.

mod errors;
mod events;
mod types;

use errors::VerificationError;
use types::{ContractHealth, DataKey, Milestone, Validator, ValidatorStatus};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

use scoutchain_shared_types::validate_cid;

const MAX_CREDENTIALS_LEN: u32 = 256;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Generated client for the progress contract — used for cross-contract calls.
// The progress contract must be deployed and its address registered via
// `set_progress_contract` before `approve_milestone` can advance levels.
mod progress_contract {
    use scoutchain_shared_types::ProgressLevel;
    use soroban_sdk::{contractclient, contracterror, Address, Env};

    #[contracterror]
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(u32)]
    pub enum Error {
        AlreadyInitialized = 1,
        NotInitialized = 2,
        ContractPaused = 3,
        Unauthorized = 4,
        InvalidProgressTransition = 5,
        AlreadyAtMaxLevel = 6,
        PlayerNotFound = 7,
    }

    #[contractclient(name = "Client")]
    #[allow(dead_code)]
    pub trait ProgressContractClient {
        fn advance_level(
            env: Env,
            caller: Address,
            player_id: u64,
            milestone_ref: u32,
        ) -> Result<ProgressLevel, Error>;
    }
}

#[contract]
pub struct VerificationContract;

#[contractimpl]
impl VerificationContract {
    // -------------------------------------------------------------------------
    // Admin
    // -------------------------------------------------------------------------

    pub fn initialize(env: Env, admin: Address) -> Result<(), VerificationError> {
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(VerificationError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_initialized(&env, &admin);
        Ok(())
    }

    /// Store the progress contract address so approve_milestone can call it.
    /// Must be called after both contracts are deployed (admin only).
    /// Returns AlreadyConfigured if called more than once — use update_progress_contract instead.
    pub fn set_progress_contract(
        env: Env,
        progress_contract: Address,
    ) -> Result<(), VerificationError> {
        Self::require_admin(&env)?;
        if env.storage().instance().has(&DataKey::ProgressContractSet) {
            return Err(VerificationError::AlreadyConfigured);
        }
        env.storage()
            .instance()
            .set(&DataKey::ProgressContract, &progress_contract);
        env.storage()
            .instance()
            .set(&DataKey::ProgressContractSet, &true);
        events::progress_contract_updated(&env, &progress_contract);
        Ok(())
    }

    /// Update the progress contract address (admin only).
    /// Use this for intentional re-wiring after the initial set_progress_contract call.
    pub fn update_progress_contract(
        env: Env,
        progress_contract: Address,
    ) -> Result<(), VerificationError> {
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::ProgressContract, &progress_contract);
        events::progress_contract_updated(&env, &progress_contract);
        Ok(())
    }

    /// Register a trusted validator (admin only).
    pub fn register_validator(
        env: Env,
        wallet: Address,
        credentials: String,
    ) -> Result<(), VerificationError> {
        Self::require_admin(&env)?;
        Self::require_not_paused(&env)?;

        if credentials.len() > MAX_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Validator(wallet.clone()))
        {
            return Err(VerificationError::ValidatorAlreadyRegistered);
        }

        let mut validator_vector: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));

        if validator_vector.len() >= 100 {
            return Err(VerificationError::ValidatorCapacityReached);
        }

        let validator = Validator {
            wallet: wallet.clone(),
            credentials,
            registered_at: env.ledger().timestamp(),
            active: true,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Validator(wallet.clone()), &validator);
        events::validator_registered(&env, &wallet);

        if !validator_vector.contains(&wallet) {
            validator_vector.push_back(wallet.clone());
        }

        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validator_vector);

        Ok(())
    }
    pub fn get_validators(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Deactivate a validator (admin only).
    /// Optionally accepts a reason (max 128 bytes) that is included in the event.
    pub fn revoke_validator(
        env: Env,
        wallet: Address,
        reason: Option<String>,
    ) -> Result<(), VerificationError> {
        Self::require_admin(&env)?;

        if let Some(ref r) = reason {
            if r.len() > 128 {
                return Err(VerificationError::ReasonTooLong);
            }
        }

        let mut validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        validator.active = false;
        env.storage()
            .persistent()
            .set(&DataKey::Validator(wallet.clone()), &validator);
        events::validator_revoked(&env, &wallet, &reason.unwrap_or(String::from_str(&env, "")));
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), VerificationError> {
        Self::require_admin(&env)?;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;

        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), VerificationError> {
        Self::require_admin(&env)?;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;

        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Milestone approval
    // -------------------------------------------------------------------------

    /// Approve a player milestone. Caller must be a registered, active validator.
    ///
    /// After storing the milestone, this function calls `progress.advance_level`
    /// on the registered progress contract so both state changes happen atomically
    /// in the same Stellar transaction.
    ///
    /// Each milestone records the Stellar ledger sequence number for
    /// tamper-proof auditability.
    ///
    /// Returns the milestone index for this player.
    pub fn approve_milestone(
        env: Env,
        validator_wallet: Address,
        player_id: u64,
        description: String,
        evidence_hash: String,
    ) -> Result<u32, VerificationError> {
        Self::require_not_paused(&env)?;
        validator_wallet.require_auth();

        validate_cid(&evidence_hash).map_err(|_| VerificationError::InvalidInput)?;

        // Verify the caller is an active validator
        let validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(validator_wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;

        if !validator.active {
            return Err(VerificationError::ValidatorInactive);
        }

        // Increment milestone counter for this player
        let counter_key = DataKey::MilestoneCounter(player_id);
        let index: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0u32);
        let next_index = index.checked_add(1).ok_or(VerificationError::Overflow)?;

        let _description_for_event = description.clone();
        let _evidence_hash_for_event = evidence_hash.clone();

        let milestone = Milestone {
            player_id,
            validator: validator_wallet.clone(),
            description: description.clone(),
            evidence_hash: evidence_hash.clone(),
            approved_at: env.ledger().timestamp(),
            ledger_sequence: env.ledger().sequence(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Milestone(player_id, next_index), &milestone);
        env.storage().persistent().set(&counter_key, &next_index);

        // Increment per-validator milestone count
        let val_key = DataKey::ValidatorMilestoneCount(validator_wallet.clone());
        let val_count: u32 = env.storage().persistent().get(&val_key).unwrap_or(0u32);
        env.storage()
            .persistent()
            .set(&val_key, &(val_count.checked_add(1).expect("overflow")));

        events::milestone_approved(
            &env,
            player_id,
            &validator_wallet,
            next_index,
            &description,
            &evidence_hash,
        );

        // Cross-contract call: advance the player's progress level.
        // This is a best-effort call — if the progress contract is not set
        // (e.g. during testing without a full deployment), we skip it.
        // In production, always call set_progress_contract before going live.
        if let Some(progress_addr) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ProgressContract)
        {
            let progress_client = progress_contract::Client::new(&env, &progress_addr);
            // AlreadyAtMaxLevel (6) is acceptable — milestone still recorded.
            // Any other error propagates as ProgressCallFailed.
            match progress_client.try_advance_level(&validator_wallet, &player_id, &next_index) {
                Ok(_) => {}
                Err(Ok(progress_contract::Error::AlreadyAtMaxLevel)) => {}
                Err(_) => return Err(VerificationError::ProgressCallFailed),
            }
        }

        Ok(next_index)
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    pub fn get_milestone(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<Milestone, VerificationError> {
        env.storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, index))
            .ok_or(VerificationError::MilestoneNotFound)
    }

    pub fn get_milestone_count(env: Env, player_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MilestoneCounter(player_id))
            .unwrap_or(0u32)
    }

    pub fn get_validator_milestone_count(env: Env, wallet: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorMilestoneCount(wallet))
            .unwrap_or(0u32)
    }

    pub fn get_validator(env: Env, wallet: Address) -> Result<Validator, VerificationError> {
        env.storage()
            .persistent()
            .get(&DataKey::Validator(wallet))
            .ok_or(VerificationError::ValidatorNotFound)
    }

    /// Returns the detailed status of a validator wallet.
    pub fn get_validator_status(env: Env, wallet: Address) -> ValidatorStatus {
        match env
            .storage()
            .persistent()
            .get::<DataKey, Validator>(&DataKey::Validator(wallet))
        {
            None => ValidatorStatus::NotRegistered,
            Some(v) if v.active => ValidatorStatus::Active,
            Some(_) => ValidatorStatus::Revoked,
        }
    }

    /// Deprecated: use `get_validator_status` instead.
    /// Returns true only for registered, active validators.
    pub fn is_active_validator(env: Env, wallet: Address) -> bool {
        Self::get_validator_status(env, wallet) == ValidatorStatus::Active
    }

    pub fn health(env: Env) -> ContractHealth {
        let initialized = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false);
        let paused = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Paused)
            .unwrap_or(false);
        ContractHealth {
            initialized,
            paused,
        }
    }

    /// Returns the deployed crate version (from Cargo.toml at build time).
    pub fn version(env: Env) -> String {
        String::from_str(&env, CONTRACT_VERSION)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn require_admin(env: &Env) -> Result<(), VerificationError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), VerificationError> {
        if env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(VerificationError::ContractPaused);
        }
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events, Ledger},
        Env, IntoVal, String, Symbol,
    };

    fn setup() -> (Env, VerificationContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| {
            l.sequence_number = 1;
        });
        let id = env.register_contract(None, VerificationContract);
        let client = VerificationContractClient::new(&env, &id);
        (env, client)
    }

    #[test]
    fn test_validator_milestone_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));

        // Unknown wallet returns 0
        assert_eq!(
            client.get_validator_milestone_count(&Address::generate(&env)),
            0
        );

        for i in 1u64..=3 {
            client.approve_milestone(
                &validator,
                &i,
                &String::from_str(&env, "milestone"),
                &String::from_str(&env, "QmEvidence"),
            );
        }

        assert_eq!(client.get_validator_milestone_count(&validator), 3);
    }

    #[test]
    fn test_health_false_before_initialize() {
        let (_env, client) = setup();
        assert!(!client.health().initialized);
    }

    #[test]
    fn test_version() {
        let (env, client) = setup();
        assert_eq!(client.version(), String::from_str(&env, "0.1.0"));
    }

    #[test]
    fn test_register_and_approve() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"));

        assert!(client.is_active_validator(&validator));

        // No progress contract set — approve_milestone still records the milestone
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Scored 5 goals in Local Cup"),
            &String::from_str(&env, "QmEvidence123"),
        );
        assert_eq!(idx, 1);
        assert_eq!(client.get_milestone_count(&1u64), 1);

        let milestone = client.get_milestone(&1u64, &1);
        assert!(milestone.ledger_sequence > 0);
    }

    #[test]
    fn test_multiple_milestones_same_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));

        let idx1 = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, "QmKYC"),
        );
        let idx2 = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, "QmSpeed"),
        );
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(client.get_milestone_count(&1u64), 2);
    }

    #[test]
    fn test_revoke_validator() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));
        let reason: Option<String> = None;
        client.revoke_validator(&validator, &reason);

        assert!(!client.is_active_validator(&validator));
    }

    #[test]
    fn test_revoke_validator_with_reason() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));
        let reason = Some(String::from_str(&env, "Misconduct and protocol violation"));
        client.revoke_validator(&validator, &reason);

        assert!(!client.is_active_validator(&validator));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn test_revoke_validator_reason_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));
        // 129-byte string
        let long_reason = "x".repeat(129);
        let reason = Some(String::from_str(&env, &long_reason));
        client.revoke_validator(&validator, &reason);
    }

    #[test]
    #[should_panic]
    fn test_revoked_validator_cannot_approve() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));
        let reason: Option<String> = None;
        client.revoke_validator(&validator, &reason);

        // Should panic — validator is inactive
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, "QmEvidence"),
        );
    }

    #[test]
    #[should_panic]
    fn test_unregistered_validator_cannot_approve() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let random = Address::generate(&env);
        // Should panic — not in validator registry
        client.approve_milestone(
            &random,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, "QmEvidence"),
        );
    }

    #[test]
    fn test_two_validators_approve_milestones_for_same_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator1 = Address::generate(&env);
        let validator2 = Address::generate(&env);
        client.register_validator(&validator1, &String::from_str(&env, "Coach A"));
        client.register_validator(&validator2, &String::from_str(&env, "Coach B"));

        client.approve_milestone(
            &validator1,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, "QmEvidence1"),
        );
        client.approve_milestone(
            &validator2,
            &1u64,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, "QmEvidence2"),
        );

        assert_eq!(client.get_milestone_count(&1u64), 2);

        let m1 = client.get_milestone(&1u64, &1);
        let m2 = client.get_milestone(&1u64, &2);
        assert_eq!(m1.validator, validator1);
        assert_eq!(m2.validator, validator2);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")]
    fn test_approve_milestone_blocked_when_paused() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));

        client.pause_contract();

        // Should panic — contract is paused
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, "QmEvidence"),
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_approve_milestone_overflow() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));

        // Pre-set the counter to u32::MAX so the next increment overflows
        env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .set(&DataKey::MilestoneCounter(1u64), &u32::MAX);
        });

        // Should return Overflow (#13) instead of panicking with expect()
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "overflow test"),
            &String::from_str(&env, "QmHash"),
        );
    }

    #[test]
    fn test_pause_unpause_events() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        client.pause_contract();
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "contract_paused"),).into_val(&env),
                    admin.clone().into_val(&env)
                )
            ]
        );

        client.unpause_contract();
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "contract_unpaused"),).into_val(&env),
                    admin.clone().into_val(&env)
                )
            ]
        );
    }

    #[test]
    #[should_panic]
    fn test_get_validator_not_found() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let unknown = Address::generate(&env);
        client.get_validator(&unknown);
    }

    #[test]
    fn test_set_progress_contract_second_call_returns_already_configured() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let addr = Address::generate(&env);
        client.set_progress_contract(&addr);

        let result = client.try_set_progress_contract(&addr);
        assert_eq!(result, Err(Ok(VerificationError::AlreadyConfigured)));
    }

    #[test]
    fn test_set_progress_contract_emits_event() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let addr = Address::generate(&env);
        client.set_progress_contract(&addr);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "progress_contract_updated"),).into_val(&env),
                    addr.into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_update_progress_contract_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let addr1 = Address::generate(&env);
        let addr2 = Address::generate(&env);
        client.set_progress_contract(&addr1);
        client.update_progress_contract(&addr2);
    }

    // -------------------------------------------------------------------------
    // Credentials length boundary tests (MAX_CREDENTIALS_LEN = 256)
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_register_validator_credentials_257_bytes_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        // 257 ASCII bytes — must exceed the 256-byte limit
        let too_long = "a".repeat(257);
        client.register_validator(&validator, &String::from_str(&env, &too_long));
    }

    #[test]
    fn test_register_validator_credentials_256_bytes_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        // Exactly 256 ASCII bytes — must be accepted
        let exactly_256 = "a".repeat(256);
        client.register_validator(&validator, &String::from_str(&env, &exactly_256));

        assert!(client.is_active_validator(&validator));
    }

    #[test]
    fn test_register_validator_capacity_returns_typed_error() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let mut validators = Vec::new(&env);
        for _ in 0..100 {
            validators.push_back(Address::generate(&env));
        }
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validators);

        let overflow_validator = Address::generate(&env);
        let result =
            client.try_register_validator(&overflow_validator, &String::from_str(&env, "Coach"));

        assert_eq!(result, Err(Ok(VerificationError::ValidatorCapacityReached)));
        assert_eq!(
            client.try_get_validator(&overflow_validator),
            Err(Ok(VerificationError::ValidatorNotFound))
        );
    }

    #[test]
    fn test_initialize_emits_contract_initialized_event() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "contract_initialized"),).into_val(&env),
                    admin.into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_duplicate_initialize_emits_no_event() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Clear events after first initialize
        let _ = env.events().all();

        // Second initialize must fail and emit no event
        let result = client.try_initialize(&admin);
        assert!(result.is_err());
        assert_eq!(env.events().all(), soroban_sdk::vec![&env]);
    }

    #[test]
    fn test_get_validators_includes_revoked() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let v1 = Address::generate(&env);
        let v2 = Address::generate(&env);
        let v3 = Address::generate(&env);

        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"));
        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"));
        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"));

        let reason: Option<String> = None;
        client.revoke_validator(&v2, &reason);

        let validators = client.get_validators();
        assert_eq!(validators.len(), 3);
        assert!(validators.contains(&v1));
        assert!(validators.contains(&v2));
        assert!(validators.contains(&v3));
    }
}
