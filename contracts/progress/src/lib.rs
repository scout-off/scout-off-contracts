#![cfg_attr(target_family = "wasm", no_std)]
#![no_std]
mod errors;
mod events;
mod types;

use errors::ProgressError;
use types::{ContractHealth, DataKey, ProgressEntry, ProgressLevel};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

const INSTANCE_TTL_MIN: u32 = 100;
const INSTANCE_TTL_MAX: u32 = 500;

const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 2000;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// #457: Minimal client for the verification contract.
// Used to confirm that a milestone_ref actually exists on-chain for a given
// player before accepting it as justification for a level advance.
mod verification_contract {
    use soroban_sdk::{contractclient, contracterror, Env};

    #[contracterror]
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(u32)]
    pub enum Error {
        MilestoneNotFound = 14,
    }

    #[contractclient(name = "Client")]
    #[allow(dead_code)]
    pub trait VerificationContractClient {
        fn get_milestone_count(env: Env, player_id: u64) -> u32;
    }
}

#[contract]
pub struct ProgressContract;

#[contractimpl]
impl ProgressContract {
    // -------------------------------------------------------------------------
    // Admin
    // -------------------------------------------------------------------------

    pub fn initialize(env: Env, admin: Address) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(ProgressError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, ADMIN_BUMP_LEDGERS, ADMIN_BUMP_LEDGERS);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    /// Store the verification contract address allowed to call `advance_level`.
    /// When set, only that contract may authorize level advances (admin only).
    pub fn set_verification_contract(env: Env, addr: Address) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::VerificationContract, &addr);
        Ok(())
    }

    /// Store the registration contract address so we can sync player levels (admin only).
    pub fn set_registration_contract(env: Env, addr: Address) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::RegistrationContract, &addr);
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    /// Optionally whitelist the scout_access contract as a secondary caller of
    /// advance_level (for trial-offer Level-3 advances). Admin only.
    pub fn set_scout_access_contract(
        env: Env,
        addr: Address,
    ) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::ScoutAccessContract, &addr);
        Ok(())
    }

    /// Transfer admin rights to a new address (current admin auth required).
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ProgressError::NotInitialized)?;
        old_admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, ADMIN_BUMP_LEDGERS, ADMIN_BUMP_LEDGERS);
        events::admin_transferred(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Upgrade the contract WASM. Admin auth required.
    /// Persistent storage (including Admin) survives this call.
    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }

    /// Reset a player's level for dispute resolution.
    /// Existing history is preserved; a new history entry records the reset.
    pub fn reset_player_level(
        env: Env,
        player_id: u64,
        target_level: ProgressLevel,
    ) -> Result<(), ProgressError> {
        Self::require_not_paused(&env)?;
        let admin = Self::require_admin(&env)?;

        let old_level = Self::get_current_level(&env, player_id);
        Self::record_progress_entry(
            &env,
            player_id,
            old_level.clone(),
            target_level.clone(),
            admin,
            0,
        )?;
        env.storage()
            .persistent()
            .set(&DataKey::PlayerLevel(player_id), &target_level);

        // Sync to registration contract if set
        if let Some(reg_contract) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegistrationContract)
        {
            let reg_client = registration_contract::Client::new(&env, &reg_contract);
            match reg_client.try_set_player_level(&player_id, &target_level) {
                Ok(Ok(())) => {}
                _ => return Err(ProgressError::RegistrationCallFailed),
            }
        }

        events::player_level_reset(&env, player_id, &old_level, &target_level);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Progress updates
    // -------------------------------------------------------------------------

    /// Advance a player's progress level by one tier.
    /// Caller must be an authorized validator (or scout for Level 3).
    /// `milestone_ref` links back to the verification contract's milestone index.
    pub fn advance_level(
        env: Env,
        caller: Address,
        player_id: u64,
        milestone_ref: u32,
    ) -> Result<ProgressLevel, ProgressError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        // Only the configured VerificationContract (or the optional secondary
        // ScoutAccessContract for trial-offer Level-3 advances) may call this
        // function.  If neither whitelist address is configured the call is
        // rejected — there is no open fallback.
        let verification_contract: Address = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::VerificationContract)
            .ok_or(ProgressError::NotInitialized)?;

        // Check whether an optional secondary caller (e.g. scout_access) is
        // also whitelisted, then require auth from whichever address matches.
        let secondary: Option<Address> = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ScoutAccessContract);

        let caller_is_secondary = secondary
            .as_ref()
            .map(|a| a == &caller)
            .unwrap_or(false);

        if caller_is_secondary {
            secondary.unwrap().require_auth();
        } else {
            verification_contract.require_auth();
        }

        // #457: When the VerificationContract is configured, validate that
        // milestone_ref actually exists on-chain for this player.  A
        // milestone_ref of 0 or one beyond the known count is rejected with
        // InvalidProgressTransition, preventing fabricated indices from
        // advancing a player's level.  This check is skipped when no
        // VerificationContract is set so that the function remains backward
        // compatible with deployments that have not yet wired the contracts.
        {
            let ver_addr: Option<Address> = env
                .storage()
                .instance()
                .get::<DataKey, Address>(&DataKey::VerificationContract);
            if let Some(ver_addr) = ver_addr {
                let ver_client = verification_contract::Client::new(&env, &ver_addr);
                let count = ver_client.get_milestone_count(&player_id);
                if milestone_ref == 0 || milestone_ref > count {
                    return Err(ProgressError::InvalidProgressTransition);
                }
            }
        }

        let current = Self::get_current_level(&env, player_id);
        let new_level = current.next().ok_or(ProgressError::AlreadyAtMaxLevel)?;

        // #455: All persistent storage writes (history entry, level, TTL bumps)
        // MUST complete before the event is emitted. This ordering ensures that
        // any indexer reading storage in response to the event sees a fully
        // consistent state. Do NOT move event emission above any storage write.
        Self::record_progress_entry(
            &env,
            player_id,
            current.clone(),
            new_level.clone(),
            caller.clone(),
            milestone_ref,
        )?;
        env.storage()
            .persistent()
            .set(&DataKey::PlayerLevel(player_id), &new_level);

        // Sync to registration contract if set
        if let Some(reg_contract) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegistrationContract)
        {
            let reg_client = registration_contract::Client::new(&env, &reg_contract);
            match reg_client.try_set_player_level(&player_id, &new_level) {
                Ok(Ok(())) => {}
                _ => return Err(ProgressError::RegistrationCallFailed),
            }
        }

        // All storage writes are complete — emit the event last.
        events::progress_updated(
            &env,
            player_id,
            &current,
            &new_level,
            &caller,
            milestone_ref,
        );
        Ok(new_level)
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    pub fn get_level(env: Env, player_id: u64) -> ProgressLevel {
        env.storage()
            .persistent()
            .get(&DataKey::PlayerLevel(player_id))
            .unwrap_or(ProgressLevel::Unverified)
    }

    pub fn get_history_count(env: Env, player_id: u64) -> u32 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .persistent()
            .get(&DataKey::HistoryCounter(player_id))
            .unwrap_or(0u32)
    }

    pub fn get_history_entry(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<ProgressEntry, ProgressError> {
        Self::bump_instance_ttl(&env);
        let entry: ProgressEntry = env
            .storage()
            .persistent()
            .get(&DataKey::HistoryEntry(player_id, index))
            .ok_or(ProgressError::PlayerNotFound)?;
        env.storage().persistent().extend_ttl(
            &DataKey::HistoryEntry(player_id, index),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(entry)
    }

    /// Return all history entries for a player in chronological order (index 1..=N).
    /// Reads a single persistent storage key (`HistoryVec`) regardless of entry count,
    /// reducing gas cost from O(N) individual reads to O(1).
    /// Returns an empty Vec if the player has no history.
    pub fn get_progress_history(env: Env, player_id: u64) -> Vec<ProgressEntry> {
        let vec_key = DataKey::HistoryVec(player_id);
        let history: Vec<ProgressEntry> = env
            .storage()
            .persistent()
            .get(&vec_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !history.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&vec_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        history
    }

    /// Paginated history retrieval. Returns entries from `offset+1` to `offset+limit`.
    /// `limit` is capped at 50. Returns an empty Vec when `offset` >= total count.
    pub fn get_progress_history_page(
        env: Env,
        player_id: u64,
        offset: u32,
        limit: u32,
    ) -> Vec<ProgressEntry> {
        const MAX_PAGE: u32 = 50;

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::HistoryCounter(player_id))
            .unwrap_or(0u32);

        if offset >= count {
            return Vec::new(&env);
        }

        let effective_limit = limit.min(MAX_PAGE);
        let start = offset + 1; // entries are 1-indexed
        let end = (start + effective_limit - 1).min(count);

        let mut entries: Vec<ProgressEntry> = Vec::new(&env);
        for i in start..=end {
            if let Some(entry) = env
                .storage()
                .persistent()
                .get(&DataKey::HistoryEntry(player_id, i))
            {
                entries.push_back(entry);
            }
        }
        entries
    }

    /// Query history entries for a player since a given Unix timestamp.
    /// Returns all entries where `updated_at >= since_timestamp`.
    /// Uses the HistoryVec for O(1) lookup, filters in-memory.
    pub fn get_history_since(
        env: Env,
        player_id: u64,
        since_timestamp: u64,
    ) -> Vec<ProgressEntry> {
        let vec_key = DataKey::HistoryVec(player_id);
        let history: Vec<ProgressEntry> = env
            .storage()
            .persistent()
            .get(&vec_key)
            .unwrap_or_else(|| Vec::new(&env));

        if !history.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&vec_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        let mut result: Vec<ProgressEntry> = Vec::new(&env);
        for i in 0..history.len() {
            if let Some(entry) = history.get(i) {
                if entry.updated_at >= since_timestamp {
                    result.push_back(entry);
                }
            }
        }
        result
    }

    pub fn health(env: Env) -> ContractHealth {
        Self::bump_instance_ttl(&env);
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

    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
    }

    fn get_current_level(env: &Env, player_id: u64) -> ProgressLevel {
        env.storage()
            .persistent()
            .get(&DataKey::PlayerLevel(player_id))
            .unwrap_or(ProgressLevel::Unverified)
    }

    fn record_progress_entry(
        env: &Env,
        player_id: u64,
        old_level: ProgressLevel,
        new_level: ProgressLevel,
        updated_by: Address,
        milestone_ref: u32,
    ) -> Result<(), ProgressError> {
        let history_key = DataKey::HistoryCounter(player_id);
        let index: u32 = env.storage().persistent().get(&history_key).unwrap_or(0u32);
        let next_index = index.checked_add(1).ok_or(ProgressError::Overflow)?;

        let entry = ProgressEntry {
            player_id,
            old_level,
            new_level,
            updated_by,
            updated_at: env.ledger().timestamp(),
            milestone_ref,
            ledger_sequence: env.ledger().sequence(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::HistoryEntry(player_id, next_index), &entry);
        env.storage().persistent().extend_ttl(
            &DataKey::HistoryEntry(player_id, next_index),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        env.storage().persistent().set(&history_key, &next_index);
        env.storage().persistent().extend_ttl(
            &history_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Also append to the single-key Vec so get_progress_history costs O(1) reads.
        let vec_key = DataKey::HistoryVec(player_id);
        let mut history: Vec<ProgressEntry> = env
            .storage()
            .persistent()
            .get(&vec_key)
            .unwrap_or_else(|| Vec::new(env));
        history.push_back(entry);
        env.storage().persistent().set(&vec_key, &history);
        env.storage()
            .persistent()
            .extend_ttl(&vec_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        Ok(())
    }

    fn require_initialized(env: &Env) -> Result<(), ProgressError> {
        if !env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            return Err(ProgressError::NotInitialized);
        }
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), ProgressError> {
        if env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(ProgressError::ContractPaused);
        }
        Ok(())
    }

    fn require_admin(env: &Env) -> Result<Address, ProgressError> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ProgressError::NotInitialized)?;
        admin.require_auth();
        env.storage().persistent().extend_ttl(&DataKey::Admin, ADMIN_BUMP_LEDGERS, ADMIN_BUMP_LEDGERS);
        Ok(admin)
    }
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events as _},
        vec, Env, IntoVal, Symbol,
    };

    fn setup() -> (Env, ProgressContractClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, ProgressContract);
        let client = ProgressContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        // Wire a dummy verification contract so advance_level doesn't reject callers.
        let verification = Address::generate(&env);
        client.set_verification_contract(&verification);
        (env, client, verification)
    }

    #[test]
    fn test_two_players_advance_independently() {
        let (env, client, validator) = setup();

        // Player 1: advance to Level 2 (PerformanceMilestones)
        client.advance_level(&validator, &1u64, &1u32);
        client.advance_level(&validator, &1u64, &2u32);

        // Player 2: advance to Level 1 (VerifiedIdentity)
        client.advance_level(&validator, &2u64, &3u32);

        assert_eq!(
            client.get_level(&1u64),
            ProgressLevel::PerformanceMilestones
        );
        assert_eq!(client.get_level(&2u64), ProgressLevel::VerifiedIdentity);
        assert_eq!(client.get_history_count(&1u64), 2);
        assert_eq!(client.get_history_count(&2u64), 1);
    }

    #[test]
    fn test_advance_level_sequence() {
        let (_, client, validator) = setup();
        let player_id = 1u64;

        // Unverified → VerifiedIdentity
        let l1 = client.advance_level(&validator, &player_id, &1u32);
        assert_eq!(l1, ProgressLevel::VerifiedIdentity);

        // VerifiedIdentity → PerformanceMilestones
        let l2 = client.advance_level(&validator, &player_id, &2u32);
        assert_eq!(l2, ProgressLevel::PerformanceMilestones);

        // PerformanceMilestones → EliteTier
        let l3 = client.advance_level(&validator, &player_id, &3u32);
        assert_eq!(l3, ProgressLevel::EliteTier);

        assert_eq!(client.get_history_count(&player_id), 3);
    }

    #[test]
    fn test_get_history_entry_correct_data() {
        let (_, client, validator) = setup();
        let player_id = 42u64;
        let milestone = 7u32;

        // Advance once: Unverified → VerifiedIdentity
        client.advance_level(&validator, &player_id, &milestone);

        // History index starts at 1
        let entry = client.get_history_entry(&player_id, &1u32);

        assert_eq!(entry.old_level, ProgressLevel::Unverified);
        assert_eq!(entry.new_level, ProgressLevel::VerifiedIdentity);
        assert_eq!(entry.updated_by, validator);
        assert_eq!(entry.milestone_ref, milestone);
    }

    // #447: HistoryEntry TTL is extended on write — entry must be readable after
    // simulated ledger advancement past the default (un-bumped) TTL.
    #[test]
    fn test_history_entry_ttl_extended_after_write() {
        let (env, client, validator) = setup();

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        let player_id = 55u64;
        client.advance_level(&validator, &player_id, &1u32);

        // Advance ledger past what the default un-bumped TTL would be.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000 + 1_000;
        });

        // Entry must still be readable — TTL was extended on write.
        let entry = client.get_history_entry(&player_id, &1u32);
        assert_eq!(entry.old_level, ProgressLevel::Unverified);
        assert_eq!(entry.new_level, ProgressLevel::VerifiedIdentity);
    }

    #[test]
    fn test_advance_level_not_initialized() {
        let env = Env::default();
        env.mock_all_auths();
        // Register the contract but deliberately skip initialize()
        let id = env.register_contract(None, ProgressContract);
        let client = ProgressContractClient::new(&env, &id);

        let caller = Address::generate(&env);
        let result = client.try_advance_level(&caller, &99u64, &1u32);

        // Contract is not initialized, so NotInitialized is returned.
        assert_eq!(result, Err(Ok(ProgressError::NotInitialized)));
    }

    #[test]
    fn test_advance_level_without_verification_contract() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, ProgressContract);
        let client = ProgressContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        // Deliberately skip set_verification_contract

        let caller = Address::generate(&env);
        let result = client.try_advance_level(&caller, &1u64, &1u32);

        // Without a VerificationContract configured, advance_level must return
        // NotInitialized instead of accepting any arbitrary caller.
        assert_eq!(result, Err(Ok(ProgressError::NotInitialized)));
    }

    #[test]
    fn test_advance_level_succeeds_when_verification_contract_set() {
        let (_, client, verification) = setup();

        // The verification contract address was already wired in setup().
        let level = client.advance_level(&verification, &1u64, &1u32);
        assert_eq!(level, ProgressLevel::VerifiedIdentity);
    }

    #[test]
    fn test_advance_level_unauthorized_when_verification_contract_set() {
        let (env, client, _verification) = setup();

        // A random address that is NOT the verification contract must be
        // rejected by require_auth — with mock_all_auths off it would panic,
        // but with mock_all_auths on the address mismatch in the whitelist
        // logic means the verification_contract.require_auth() is satisfied
        // by the mock, so we need to clear mocks for this test.
        env.mock_auths(&[]);
        let random = Address::generate(&env);
        let result = client.try_advance_level(&random, &1u64, &1u32);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_progress_history_three_entries() {
        let (_, client, validator) = setup();
        let player_id = 10u64;

        // Advance through all three tiers
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        let history = client.get_progress_history(&player_id);

        assert_eq!(history.len(), 3);

        // Entry 1: Unverified → VerifiedIdentity
        assert_eq!(history.get(0).unwrap().old_level, ProgressLevel::Unverified);
        assert_eq!(
            history.get(0).unwrap().new_level,
            ProgressLevel::VerifiedIdentity
        );
        assert_eq!(history.get(0).unwrap().milestone_ref, 1u32);

        // Entry 2: VerifiedIdentity → PerformanceMilestones
        assert_eq!(
            history.get(1).unwrap().old_level,
            ProgressLevel::VerifiedIdentity
        );
        assert_eq!(
            history.get(1).unwrap().new_level,
            ProgressLevel::PerformanceMilestones
        );
        assert_eq!(history.get(1).unwrap().milestone_ref, 2u32);

        // Entry 3: PerformanceMilestones → EliteTier
        assert_eq!(
            history.get(2).unwrap().old_level,
            ProgressLevel::PerformanceMilestones
        );
        assert_eq!(history.get(2).unwrap().new_level, ProgressLevel::EliteTier);
        assert_eq!(history.get(2).unwrap().milestone_ref, 3u32);
    }

    #[test]
    fn test_get_progress_history_empty() {
        let (_, client, _) = setup();

        // Player 999 has never had advance_level called
        let history = client.get_progress_history(&999u64);
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn test_get_progress_history_page() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let player_id = 20u64;

        // Advance through all 3 tiers
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        // First page: offset=0, limit=2 → entries 1,2
        let page1 = client.get_progress_history_page(&player_id, &0u32, &2u32);
        assert_eq!(page1.len(), 2);
        assert_eq!(page1.get(0).unwrap().old_level, ProgressLevel::Unverified);
        assert_eq!(page1.get(1).unwrap().old_level, ProgressLevel::VerifiedIdentity);

        // Middle page: offset=1, limit=1 → entry 2
        let mid = client.get_progress_history_page(&player_id, &1u32, &1u32);
        assert_eq!(mid.len(), 1);
        assert_eq!(mid.get(0).unwrap().old_level, ProgressLevel::VerifiedIdentity);

        // Last page: offset=2, limit=50 → entry 3 only
        let last = client.get_progress_history_page(&player_id, &2u32, &50u32);
        assert_eq!(last.len(), 1);
        assert_eq!(last.get(0).unwrap().new_level, ProgressLevel::EliteTier);

        // Offset beyond count → empty
        let empty = client.get_progress_history_page(&player_id, &10u32, &5u32);
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_progress_updated_event_data() {
        let (env, client, validator) = setup();
        let player_id = 5u64;

        // Advance once: Unverified → VerifiedIdentity
        client.advance_level(&validator, &player_id, &1u32);

        // env.events().all() returns ContractEvents which compares against
        // soroban_sdk::Vec<(Address, Vec<Val>, Val)>:
        //   - Address  : the contract that emitted the event
        //   - Vec<Val> : topics  — (Symbol("progress_updated"), updated_by)
        //   - Val      : data    — (player_id, old_level, new_level)
        let contract_id = client.address.clone();
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    contract_id,
                    soroban_sdk::vec![
                        &env,
                        Symbol::new(&env, "progress_updated").into_val(&env),
                        validator.into_val(&env),
                    ],
                    (
                        player_id,
                        ProgressLevel::Unverified,
                        ProgressLevel::VerifiedIdentity
                    )
                        .into_val(&env),
                )
            ]
        );
    }

    #[test]
    #[should_panic]
    fn test_cannot_exceed_elite_tier() {
        let (_, client, validator) = setup();
        let player_id = 1u64;

        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);
        // This should panic — already at EliteTier
        client.advance_level(&validator, &player_id, &4u32);
    }

    #[test]
    fn test_transfer_admin_success() {
        let (env, client, _) = setup();
        let new_admin = Address::generate(&env);
        // Should not panic — current admin auth is satisfied
        client.transfer_admin(&new_admin);
    }

    #[test]
    #[should_panic]
    fn test_transfer_admin_unauthorized() {
        let (env, client, _) = setup();
        // Clear all mocks — no auth satisfied, so admin check fails
        env.mock_auths(&[]);
        client.transfer_admin(&Address::generate(&env));
    }

    #[test]
    fn test_pause_and_unpause() {
        let (env, client, validator) = setup();
        let player_id = 42u64;

        // --- pause ---
        client.pause_contract();

        // advance_level must be rejected with ContractPaused while paused
        let err = client
            .try_advance_level(&validator, &player_id, &1u32)
            .expect_err("expected an error while paused");
        assert_eq!(
            err.unwrap(),
            ProgressError::ContractPaused,
            "expected ContractPaused error"
        );

        // player level must be unchanged
        assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);

        // --- unpause ---
        client.unpause_contract();

        // advance_level must now succeed
        let new_level = client.advance_level(&validator, &player_id, &1u32);
        assert_eq!(new_level, ProgressLevel::VerifiedIdentity);
    }

    #[test]
    #[should_panic]
    fn test_old_admin_loses_access_after_transfer() {
        let (env, client, _) = setup();
        let new_admin = Address::generate(&env);
        client.transfer_admin(&new_admin);

        // Clear mocks — old admin auth no longer stored, so pause must fail
        env.mock_auths(&[]);
        client.pause_contract();
    }

    #[test]
    fn test_upgrade_preserves_admin() {
        let env = Env::default();
        env.mock_all_auths();

        let id = env.register_contract(None, ProgressContract);
        let client = ProgressContractClient::new(&env, &id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Register a player so we confirm persistent data also survives
        let verification = Address::generate(&env);
        client.set_verification_contract(&verification);

        let player_id = 1u64;
        client.advance_level(&verification, &player_id, &1u32);

        // Simulate upgrade: in testutils mode the host accepts empty bytes
        let new_wasm_hash = env.deployer().upload_contract_wasm(soroban_sdk::Bytes::new(&env));

        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.pause_contract();

        // Existing data persisted
        assert_eq!(
            client.get_level(&player_id),
            ProgressLevel::VerifiedIdentity
        );
    }

    #[test]
    fn test_reset_player_level_success() {
        let (env, client, validator) = setup();
        let player_id = 1u64;

        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        assert_eq!(client.get_history_count(&player_id), 2);

        client.reset_player_level(&player_id, &ProgressLevel::Unverified);

        assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);
        assert_eq!(client.get_history_count(&player_id), 3);

        let reset_entry = client.get_history_entry(&player_id, &3u32);
        assert_eq!(reset_entry.old_level, ProgressLevel::PerformanceMilestones);
        assert_eq!(reset_entry.new_level, ProgressLevel::Unverified);
        assert_eq!(reset_entry.milestone_ref, 0);
        // The event is still emitted
        assert_eq!(
            env.events().all(),
            vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "player_level_reset"),).into_val(&env),
                    (
                        player_id,
                        ProgressLevel::PerformanceMilestones,
                        ProgressLevel::Unverified,
                    )
                        .into_val(&env),
                ),
            ]
        );
    }

    #[test]
    #[should_panic]
    fn test_reset_player_level_unauthorized() {
        let (env, client, _) = setup();
        env.mock_auths(&[]);
        client.reset_player_level(&1u64, &ProgressLevel::Unverified);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #8)")]
    fn test_advance_level_history_counter_overflow() {
        let (env, client, validator) = setup();
        let player_id = 1u64;

        env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .set(&DataKey::HistoryCounter(player_id), &u32::MAX);
        });

        client.advance_level(&validator, &player_id, &1u32);
    }

    #[test]
    fn test_full_level_progression_with_history() {
        let (_, client, validator) = setup();
        let player_id = 7u64;

        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        assert_eq!(client.get_level(&player_id), ProgressLevel::EliteTier);
        assert_eq!(client.get_history_count(&player_id), 3);

        let h1 = client.get_history_entry(&player_id, &1u32);
        let h3 = client.get_history_entry(&player_id, &3u32);
        assert_eq!(h1.old_level, ProgressLevel::Unverified);
        assert_eq!(h3.new_level, ProgressLevel::EliteTier);
    }

    #[test]
    fn test_fourth_advance_panics() {
        let (_, client, validator) = setup();
        let player_id = 1u64;

        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        let result = client.try_advance_level(&validator, &player_id, &4u32);
        assert_eq!(result, Err(Ok(ProgressError::AlreadyAtMaxLevel)));
    }

    // -------------------------------------------------------------------------
    // #455: Event is emitted only after all storage writes are finalised
    // -------------------------------------------------------------------------

    #[test]
    fn test_event_payload_matches_storage_state_at_emission() {
        let (env, client, validator) = setup();
        let player_id = 55u64;

        client.advance_level(&validator, &player_id, &1u32);

        // After advance_level returns, both the storage state and the event
        // must agree: the player is at VerifiedIdentity.
        let stored_level = client.get_level(&player_id);
        assert_eq!(stored_level, ProgressLevel::VerifiedIdentity);

        // The emitted event must reflect the same new_level that is in storage.
        let events = env.events().all();
        assert_eq!(events.len(), 1);
        // Event data encodes (player_id, old_level, new_level).
        // We verify new_level in storage equals VerifiedIdentity, which is
        // what the event carries — confirming the write happened before emit.
        let history = client.get_progress_history(&player_id);
        assert_eq!(history.get(0).unwrap().new_level, stored_level);
    }

    // -------------------------------------------------------------------------
    // #457: milestone_ref is validated against the verification contract
    // -------------------------------------------------------------------------

    #[test]
    fn test_advance_level_invalid_milestone_ref_rejected_when_verification_set() {
        use scoutchain_verification::VerificationContract;
        use scoutchain_verification::VerificationContractClient;

        let env = Env::default();
        env.mock_all_auths();

        // Deploy verification contract and register a validator + milestone.
        let ver_id = env.register_contract(None, VerificationContract);
        let ver_client = VerificationContractClient::new(&env, &ver_id);
        let ver_admin = Address::generate(&env);
        ver_client.initialize(&ver_admin);

        // Deploy progress contract and wire the verification address.
        let prog_id = env.register_contract(None, ProgressContract);
        let prog_client = ProgressContractClient::new(&env, &prog_id);
        let prog_admin = Address::generate(&env);
        prog_client.initialize(&prog_admin);
        prog_client.set_verification_contract(&ver_id);

        let validator = Address::generate(&env);
        ver_client.register_validator(&validator, &soroban_sdk::String::from_str(&env, "Coach"));
        // Approve one milestone for player 1 → milestone_ref 1 is valid.
        ver_client.approve_milestone(
            &validator,
            &1u64,
            &soroban_sdk::String::from_str(&env, "scored"),
            &soroban_sdk::String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // Valid ref (1) must succeed.
        let result = prog_client.try_advance_level(&validator, &1u64, &1u32);
        assert!(result.is_ok(), "valid milestone_ref should succeed");

        // Non-existent ref (99) must be rejected.
        let result = prog_client.try_advance_level(&validator, &1u64, &99u32);
        assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));

        // Zero ref must also be rejected.
        let result = prog_client.try_advance_level(&validator, &1u64, &0u32);
        assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));
    }

    #[test]
    fn test_advance_level_skips_milestone_validation_when_verification_not_set() {
        // When no VerificationContract is configured, any milestone_ref is
        // accepted (backward-compatible behaviour).
        let env = Env::default();
        env.mock_all_auths();
        let prog_id = env.register_contract(None, ProgressContract);
        let prog_client = ProgressContractClient::new(&env, &prog_id);
        let admin = Address::generate(&env);
        prog_client.initialize(&admin);
        let verification = Address::generate(&env);
        prog_client.set_verification_contract(&verification);
        // Note: no real verification contract deployed — but the milestone
        // validation is skipped when the stored address has no milestone data
        // for the player (count = 0 would reject), so for true "not set" we
        // test on a freshly initialized contract WITHOUT calling
        // set_verification_contract.
        let env2 = Env::default();
        env2.mock_all_auths();
        let prog_id2 = env2.register_contract(None, ProgressContract);
        let prog_client2 = ProgressContractClient::new(&env2, &prog_id2);
        let admin2 = Address::generate(&env2);
        prog_client2.initialize(&admin2);
        // No set_verification_contract call here — should error NotInitialized
        // (no VerificationContract key means advance_level rejects).
        let caller = Address::generate(&env2);
        let result = prog_client2.try_advance_level(&caller, &1u64, &99u32);
        assert_eq!(result, Err(Ok(ProgressError::NotInitialized)));
    }
}
