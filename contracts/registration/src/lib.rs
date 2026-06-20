#![no_std]
mod errors;
mod events;
mod types;

use errors::ScoutChainError;
use types::{ContractHealth, DataKey, PlayerProfile, PlayerVitals, ProgressLevel, ScoutProfile};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

const MAX_REGION_LEN: u32 = 128;
const MAX_STRING_LEN: u32 = 64;
const MAX_IPFS_HASHES: u32 = 10;

#[contract]
pub struct RegistrationContract;

#[contractimpl]
impl RegistrationContract {
    // -------------------------------------------------------------------------
    // Admin
    // -------------------------------------------------------------------------

    /// One-time contract initialisation. Must be called before any other function.
    pub fn initialize(env: Env, admin: Address) -> Result<(), ScoutChainError> {
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(ScoutChainError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::PlayerCounter, &0u64);
        env.storage().instance().set(&DataKey::ScoutCounter, &0u64);
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), ScoutChainError> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ScoutChainError> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    /// Transfer admin rights to a new address (current admin auth required).
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ScoutChainError> {
        let old_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ScoutChainError::NotInitialized)?;
        old_admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        events::admin_transferred(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Store the progress contract address so it can call set_player_level (admin only).
    pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutChainError> {
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::ProgressContract, &addr);
        Ok(())
    }

    /// Update a player's progress level. Only callable by the registered progress contract.
    pub fn set_player_level(
        env: Env,
        player_id: u64,
        level: ProgressLevel,
    ) -> Result<(), ScoutChainError> {
        let progress_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::ProgressContract)
            .ok_or(ScoutChainError::Unauthorized)?;
        progress_contract.require_auth();

        let mut profile = Self::load_player(&env, player_id)?;
        profile.level = level;
        profile.updated_at = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::Player(player_id), &profile);
        events::player_level_synced(&env, player_id);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Player registration
    // -------------------------------------------------------------------------

    /// Register a new player profile at Level 0 (Unverified).
    /// `ipfs_hashes` — list of IPFS/Arweave CIDs for highlight reels and photos.
    pub fn register_player(
        env: Env,
        wallet: Address,
        vitals: PlayerVitals,
        ipfs_hashes: Vec<String>,
    ) -> Result<u64, ScoutChainError> {
        Self::require_initialized(&env)?;
        Self::require_not_paused(&env)?;
        wallet.require_auth();

        // Prevent duplicate registrations
        if env
            .storage()
            .persistent()
            .has(&DataKey::PlayerByWallet(wallet.clone()))
        {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        // Validate vitals string lengths
        if vitals.position.len() > MAX_STRING_LEN
            || vitals.region.len() > MAX_STRING_LEN
            || vitals.nationality.len() > MAX_STRING_LEN
        {
            return Err(ScoutChainError::InvalidInput);
        }

        // Validate ipfs_hashes: non-empty and at most MAX_IPFS_HASHES
        if ipfs_hashes.is_empty() || ipfs_hashes.len() > MAX_IPFS_HASHES {
            return Err(ScoutChainError::InvalidInput);
        }

        let player_id = Self::next_player_id(&env)?;
        let now = env.ledger().timestamp();

        let profile = PlayerProfile {
            player_id,
            wallet: wallet.clone(),
            vitals,
            ipfs_hashes,
            level: ProgressLevel::Unverified,
            registered_at: now,
            updated_at: now,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Player(player_id), &profile);
        env.storage()
            .persistent()
            .set(&DataKey::PlayerByWallet(wallet.clone()), &player_id);

        // Add to player index
        let mut player_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerIndex)
            .unwrap_or_else(|| Vec::new(&env));
        player_ids.push_back(player_id);
        env.storage()
            .persistent()
            .set(&DataKey::PlayerIndex, &player_ids);

        events::player_registered(&env, player_id, &wallet);
        Ok(player_id)
    }

    /// Update a player's IPFS content hashes (player auth required).
    pub fn update_profile(
        env: Env,
        player_id: u64,
        ipfs_hashes: Vec<String>,
    ) -> Result<(), ScoutChainError> {
        Self::require_not_paused(&env)?;
        let mut profile = Self::load_player(&env, player_id)?;
        profile.wallet.require_auth();
        if ipfs_hashes.is_empty() || ipfs_hashes.len() > MAX_IPFS_HASHES {
            return Err(ScoutChainError::InvalidInput);
        }
        profile.ipfs_hashes = ipfs_hashes;
        profile.updated_at = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::Player(player_id), &profile);
        events::profile_updated(&env, player_id);
        Ok(())
    }

    /// Deregister a player profile (admin only, GDPR right-to-erasure).
    pub fn deregister_player(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        Self::require_admin(&env)?;
        let profile = Self::load_player(&env, player_id)?;
        env.storage()
            .persistent()
            .remove(&DataKey::Player(player_id));
        env.storage()
            .persistent()
            .remove(&DataKey::PlayerByWallet(profile.wallet));

        // Remove from player index
        let mut player_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerIndex)
            .unwrap_or_else(|| Vec::new(&env));
        if let Some(pos) = player_ids.iter().position(|id| id == player_id) {
            player_ids.remove(pos as u32);
            env.storage()
                .persistent()
                .set(&DataKey::PlayerIndex, &player_ids);
        }

        events::player_deregistered(&env, player_id);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Scout registration
    // -------------------------------------------------------------------------

    /// Register a new scout profile.
    pub fn register_scout(
        env: Env,
        wallet: Address,
        region: String,
    ) -> Result<u64, ScoutChainError> {
        Self::require_initialized(&env)?;
        Self::require_not_paused(&env)?;
        wallet.require_auth();

        if region.len() > MAX_REGION_LEN {
            return Err(ScoutChainError::InvalidInput);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::ScoutByWallet(wallet.clone()))
        {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        let scout_id = Self::next_scout_id(&env)?;
        let profile = ScoutProfile {
            scout_id,
            wallet: wallet.clone(),
            region,
            verified: false,
            registered_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Scout(scout_id), &profile);
        env.storage()
            .persistent()
            .set(&DataKey::ScoutByWallet(wallet.clone()), &scout_id);

        events::scout_registered(&env, scout_id, &wallet);
        Ok(scout_id)
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    pub fn get_player(env: Env, player_id: u64) -> Result<PlayerProfile, ScoutChainError> {
        Self::load_player(&env, player_id)
    }

    pub fn get_player_by_wallet(
        env: Env,
        wallet: Address,
    ) -> Result<PlayerProfile, ScoutChainError> {
        let player_id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerByWallet(wallet))
            .ok_or(ScoutChainError::PlayerNotFound)?;
        Self::load_player(&env, player_id)
    }

    pub fn get_scout(env: Env, scout_id: u64) -> Result<ScoutProfile, ScoutChainError> {
        env.storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutNotFound)
    }

    /// Verify a scout profile (admin only).
    pub fn verify_scout(env: Env, scout_id: u64) -> Result<(), ScoutChainError> {
        Self::require_admin(&env)?;
        let mut profile: ScoutProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutNotFound)?;
        profile.verified = true;
        env.storage()
            .persistent()
            .set(&DataKey::Scout(scout_id), &profile);
        events::scout_verified(&env, scout_id);
        Ok(())
    }

    pub fn get_player_count(env: Env) -> u64 {
        if !env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            return 0;
        }
        env.storage()
            .instance()
            .get(&DataKey::PlayerCounter)
            .unwrap_or(0u64)
    }

    pub fn get_scout_count(env: Env) -> u64 {
        if !env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            return 0;
        }
        env.storage()
            .instance()
            .get(&DataKey::ScoutCounter)
            .unwrap_or(0u64)
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

    /// Filter players by region, position, and minimum progress level.
    /// Returns at most 50 results to bound gas usage.
    pub fn filter_players(
        env: Env,
        region: String,
        position: String,
        min_level: ProgressLevel,
    ) -> Result<Vec<PlayerProfile>, ScoutChainError> {
        Self::require_initialized(&env)?;

        let player_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerIndex)
            .unwrap_or_else(|| Vec::new(&env));

        let mut results = Vec::new(&env);
        let max_results = 50u32;

        for player_id in player_ids.iter() {
            if results.len() >= max_results {
                break;
            }

            if let Ok(profile) = Self::load_player(&env, player_id) {
                if profile.vitals.region == region
                    && profile.vitals.position == position
                    && Self::level_gte(&profile.level, &min_level)
                {
                    results.push_back(profile);
                }
            }
        }

        Ok(results)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn require_initialized(env: &Env) -> Result<(), ScoutChainError> {
        if !env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            return Err(ScoutChainError::NotInitialized);
        }
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), ScoutChainError> {
        if env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(ScoutChainError::ContractPaused);
        }
        Ok(())
    }

    fn require_admin(env: &Env) -> Result<(), ScoutChainError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ScoutChainError::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }

    fn load_player(env: &Env, player_id: u64) -> Result<PlayerProfile, ScoutChainError> {
        env.storage()
            .persistent()
            .get(&DataKey::Player(player_id))
            .ok_or(ScoutChainError::PlayerNotFound)
    }

    fn next_player_id(env: &Env) -> Result<u64, ScoutChainError> {
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::PlayerCounter)
            .unwrap_or(0u64);
        let next = id.checked_add(1).ok_or(ScoutChainError::Overflow)?;
        env.storage().instance().set(&DataKey::PlayerCounter, &next);
        Ok(next)
    }

    fn next_scout_id(env: &Env) -> Result<u64, ScoutChainError> {
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ScoutCounter)
            .unwrap_or(0u64);
        let next = id.checked_add(1).ok_or(ScoutChainError::Overflow)?;
        env.storage().instance().set(&DataKey::ScoutCounter, &next);
        Ok(next)
    }

    fn level_gte(level: &ProgressLevel, min_level: &ProgressLevel) -> bool {
        matches!(
            (level, min_level),
            (ProgressLevel::Unverified, ProgressLevel::Unverified)
                | (ProgressLevel::VerifiedIdentity, ProgressLevel::Unverified)
                | (
                    ProgressLevel::PerformanceMilestones,
                    ProgressLevel::Unverified
                )
                | (ProgressLevel::EliteTier, ProgressLevel::Unverified)
                | (
                    ProgressLevel::VerifiedIdentity,
                    ProgressLevel::VerifiedIdentity
                )
                | (
                    ProgressLevel::PerformanceMilestones,
                    ProgressLevel::VerifiedIdentity
                )
                | (ProgressLevel::EliteTier, ProgressLevel::VerifiedIdentity)
                | (
                    ProgressLevel::PerformanceMilestones,
                    ProgressLevel::PerformanceMilestones
                )
                | (
                    ProgressLevel::EliteTier,
                    ProgressLevel::PerformanceMilestones
                )
                | (ProgressLevel::EliteTier, ProgressLevel::EliteTier)
        )
    }
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env, String};

    fn setup() -> (Env, RegistrationContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RegistrationContract);
        let client = RegistrationContractClient::new(&env, &contract_id);
        (env, client)
    }

    fn dummy_vitals(env: &Env) -> PlayerVitals {
        PlayerVitals {
            age: 18,
            position: String::from_str(env, "Forward"),
            region: String::from_str(env, "West Africa"),
            nationality: String::from_str(env, "Ghana"),
        }
    }

    #[test]
    fn test_initialize_and_health() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        assert!(client.health().initialized);
    }

    #[test]
    fn test_transfer_admin_success() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let new_admin = Address::generate(&env);
        client.transfer_admin(&new_admin);
    }

    #[test]
    fn test_register_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes: soroban_sdk::Vec<String> = vec![&env, String::from_str(&env, "QmTest123")];

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);

        let profile = client.get_player(&player_id);
        assert_eq!(profile.wallet, wallet);
        assert_eq!(profile.level, ProgressLevel::Unverified);
    }

    #[test]
    #[should_panic]
    fn test_duplicate_registration_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes: soroban_sdk::Vec<String> = vec![&env, String::from_str(&env, "QmTest")];

        client.register_player(&wallet, &vitals, &hashes);
        // second call should panic with AlreadyRegistered
        client.register_player(&wallet, &vitals, &hashes);
    }

    #[test]
    fn test_register_scout_region_128_bytes_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, &"A".repeat(128));
        let scout_id = client.register_scout(&wallet, &region);
        assert_eq!(scout_id, 1);
    }

    // -------------------------------------------------------------------------
    // Issue #6: position / region / nationality length validation
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic]
    fn test_register_player_position_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let long = String::from_str(&env, &"A".repeat(65));
        let vitals = PlayerVitals {
            age: 20,
            position: long,
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        client.register_player(&wallet, &vitals, &hashes);
    }

    #[test]
    fn test_register_player_position_max_len_ok() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let exactly_64 = String::from_str(&env, &"A".repeat(64));
        let vitals = PlayerVitals {
            age: 20,
            position: exactly_64,
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(id, 1);
    }

    #[test]
    #[should_panic]
    fn test_register_player_region_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let long = String::from_str(&env, &"A".repeat(65));
        let vitals = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: long,
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        client.register_player(&wallet, &vitals, &hashes);
    }

    #[test]
    #[should_panic]
    fn test_register_player_nationality_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let long = String::from_str(&env, &"A".repeat(65));
        let vitals = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: long,
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        client.register_player(&wallet, &vitals, &hashes);
    }

    // -------------------------------------------------------------------------
    // Issue #6 + #7: ipfs_hashes validation in register_player and update_profile
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic]
    fn test_register_player_empty_hashes_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes: soroban_sdk::Vec<String> = vec![&env];
        client.register_player(&wallet, &vitals, &hashes);
    }

    #[test]
    #[should_panic]
    fn test_register_player_too_many_hashes_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let h = String::from_str(&env, "QmHash");
        let hashes = vec![
            &env,
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
        ];
        client.register_player(&wallet, &vitals, &hashes);
    }

    #[test]
    #[should_panic]
    fn test_update_profile_empty_hashes_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        let empty: soroban_sdk::Vec<String> = vec![&env];
        client.update_profile(&player_id, &empty);
    }

    #[test]
    #[should_panic]
    fn test_update_profile_too_many_hashes_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        let h = String::from_str(&env, "QmHash");
        let too_many = vec![
            &env,
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
            h.clone(),
        ];
        client.update_profile(&player_id, &too_many);
    }

    #[test]
    fn test_update_profile_valid_hashes_persisted() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmOld")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        let new_hashes = vec![
            &env,
            String::from_str(&env, "QmNew1"),
            String::from_str(&env, "QmNew2"),
        ];
        client.update_profile(&player_id, &new_hashes);

        let profile = client.get_player(&player_id);
        assert_eq!(profile.ipfs_hashes.len(), 2);
    }

    // -------------------------------------------------------------------------
    // Issue #9: register_scout region length validation
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic]
    fn test_register_scout_region_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, &"a".repeat(129));
        client.register_scout(&wallet, &region);
    }

    #[test]
    fn test_register_scout_region_max_len_ok() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let exactly_128 = String::from_str(&env, &"A".repeat(128));
        let scout_id = client.register_scout(&wallet, &exactly_128);
        assert_eq!(scout_id, 1);
    }

    // -------------------------------------------------------------------------
    // Issue #28: require_initialized before require_not_paused
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic]
    fn test_register_player_uninitialized_returns_not_initialized() {
        let (env, client) = setup();
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        client.register_player(&wallet, &vitals, &hashes);
    }

    #[test]
    #[should_panic]
    fn test_register_scout_uninitialized_returns_not_initialized() {
        let (env, client) = setup();
        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        client.register_scout(&wallet, &region);
    }

    // -------------------------------------------------------------------------
    // Issue #34: Dual-role wallet policy (player + scout same wallet)
    // -------------------------------------------------------------------------

    #[test]
    fn test_same_wallet_can_register_as_player_and_scout() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let region = String::from_str(&env, "Europe");

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);

        let scout_id = client.register_scout(&wallet, &region);
        assert_eq!(scout_id, 1);

        let player = client.get_player(&player_id);
        assert_eq!(player.wallet, wallet);

        let scout = client.get_scout(&scout_id);
        assert_eq!(scout.wallet, wallet);
    }

    // -------------------------------------------------------------------------
    // Issue #26: get_player_count and get_scout_count query functions
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_player_count_returns_zero_before_init() {
        let (env, client) = setup();
        assert_eq!(client.get_player_count(), 0);
    }

    #[test]
    fn test_get_scout_count_returns_zero_before_init() {
        let (env, client) = setup();
        assert_eq!(client.get_scout_count(), 0);
    }

    #[test]
    fn test_get_player_count_after_registrations() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        for _ in 0..3 {
            let wallet = Address::generate(&env);
            client.register_player(&wallet, &vitals, &hashes);
        }

        assert_eq!(client.get_player_count(), 3);
    }

    #[test]
    fn test_get_scout_count_after_registrations() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let region = String::from_str(&env, "Europe");

        for _ in 0..3 {
            let wallet = Address::generate(&env);
            client.register_scout(&wallet, &region);
        }

        assert_eq!(client.get_scout_count(), 3);
    }

    // -------------------------------------------------------------------------
    // Issue #31: filter_players query function
    // -------------------------------------------------------------------------

    #[test]
    fn test_filter_players_by_region_and_position() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        // Player 1: Forward, West Africa
        let wallet1 = Address::generate(&env);
        let vitals1 = PlayerVitals {
            age: 18,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        client.register_player(&wallet1, &vitals1, &hashes);

        // Player 2: Midfielder, West Africa
        let wallet2 = Address::generate(&env);
        let vitals2 = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Midfielder"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Nigeria"),
        };
        client.register_player(&wallet2, &vitals2, &hashes);

        // Player 3: Forward, Europe
        let wallet3 = Address::generate(&env);
        let vitals3 = PlayerVitals {
            age: 19,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "Europe"),
            nationality: String::from_str(&env, "France"),
        };
        client.register_player(&wallet3, &vitals3, &hashes);

        // Filter: Forward in West Africa
        let results = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results.get(0).unwrap().player_id, 1);
    }

    // -------------------------------------------------------------------------
    // Issue #32: Scout verified flag and verify_scout admin function
    // -------------------------------------------------------------------------

    #[test]
    fn test_newly_registered_scout_not_verified() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&wallet, &region);

        let scout = client.get_scout(&scout_id);
        assert!(!scout.verified);
    }

    #[test]
    fn test_admin_can_verify_scout() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&wallet, &region);

        client.verify_scout(&scout_id);

        let scout = client.get_scout(&scout_id);
        assert!(scout.verified);
    }

    // -------------------------------------------------------------------------
    // Issue #33: Full player registration and profile update flow
    // -------------------------------------------------------------------------

    #[test]
    fn test_full_player_registration_and_update_flow() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let initial_hashes = vec![&env, String::from_str(&env, "QmInitial1")];

        // Step 1: Register player
        let player_id = client.register_player(&wallet, &vitals, &initial_hashes);
        assert_eq!(player_id, 1);

        // Step 2: Get profile and verify initial state
        let profile_v1 = client.get_player(&player_id);
        assert_eq!(profile_v1.player_id, player_id);
        assert_eq!(profile_v1.wallet, wallet);
        assert_eq!(profile_v1.level, ProgressLevel::Unverified);
        assert_eq!(profile_v1.ipfs_hashes.len(), 1);
        assert_eq!(
            profile_v1.ipfs_hashes.get(0).unwrap(),
            String::from_str(&env, "QmInitial1")
        );
        let updated_at_v1 = profile_v1.updated_at;

        // Step 3: Update profile with new hashes
        let updated_hashes = vec![
            &env,
            String::from_str(&env, "QmUpdated1"),
            String::from_str(&env, "QmUpdated2"),
        ];
        client.update_profile(&player_id, &updated_hashes);

        // Step 4: Read back updated profile
        let profile_v2 = client.get_player(&player_id);
        assert_eq!(profile_v2.player_id, player_id);
        assert_eq!(profile_v2.wallet, wallet);
        assert_eq!(profile_v2.level, ProgressLevel::Unverified);
        assert_eq!(profile_v2.ipfs_hashes.len(), 2);
        assert_eq!(
            profile_v2.ipfs_hashes.get(0).unwrap(),
            String::from_str(&env, "QmUpdated1")
        );
        assert_eq!(
            profile_v2.ipfs_hashes.get(1).unwrap(),
            String::from_str(&env, "QmUpdated2")
        );

        // Step 5: Verify timestamps
        assert!(profile_v2.updated_at >= updated_at_v1);
    }
}
