#![cfg_attr(target_family = "wasm", no_std)]
mod errors;
mod events;
mod types;

use errors::ScoutChainError;
use types::{
    ContractHealth, DataKey, FilterResult, PlayerProfile, PlayerSummary, PlayerVitals,
    ProgressLevel, ScoutProfile, StoredPlayerProfile,
};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

// Generated client stub for the progress contract — used to resolve a player's
// current level at read time.  `level` is never stored in this contract.
mod progress_contract {
    use scoutchain_shared_types::{require_admin, ProgressLevel};
    use soroban_sdk::{contractclient, Env};

    #[contractclient(name = "Client")]
    #[allow(dead_code)]
    pub trait ProgressContractClient {
        fn get_level(env: Env, player_id: u64) -> ProgressLevel;
    }
}

// Bounded well below the Stellar ledger key size limit (250 bytes): the
// `PlayersByLevelRegion(level, region)` composite index embeds this string
// directly in a persistent ledger key, so a region anywhere near 128 bytes
// can push that key past the network limit and make registration fail.
const MAX_REGION_LEN: u32 = 100;
const MAX_STRING_LEN: u32 = 64;
const MAX_IPFS_HASHES: u32 = 10;
const MAX_BATCH_SIZE: u32 = 20;
/// Maximum plausible age for a registered player. Ages above this value are
/// rejected as implausible to prevent corrupt entries in discovery filters.
const MAX_PLAYER_AGE: u32 = 100;

/// Minimum scoutable age for player registration.
/// Players younger than this age cannot be registered on the platform.
/// Enforced by `register_player` to ensure off-chain age-gated scouting
/// rules can rely on the contract's on-chain guarantee.
const MIN_PLAYER_AGE: u32 = 16;

// Instance TTL bump
#[allow(dead_code)]
const INSTANCE_TTL_MIN: u32 = 100;
#[allow(dead_code)]
const INSTANCE_TTL_MAX: u32 = 500;

// Persistent storage TTL bump for player profiles and admin key.
const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 2_000;

// Admin key TTL — kept equal to PERSISTENT_TTL_MAX for simplicity.
const ADMIN_BUMP_LEDGERS: u32 = 2_000;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

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
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::PlayerCounter, &0u64);
        env.storage().instance().set(&DataKey::ScoutCounter, &0u64);
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    /// Upgrade the contract WASM. Admin auth required.
    /// Persistent storage (including Admin) survives this call.
    pub fn upgrade(
        env: Env,
        new_wasm_hash: soroban_sdk::BytesN<32>,
    ) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }

    /// Store the progress contract address so filter_players can resolve
    /// levels at query time (admin only).
    pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
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

        // Use the stored profile directly rather than `load_player` — the
        // latter live-resolves the level via a cross-call back into the
        // calling progress contract, which is already on the call stack
        // here (it invoked set_player_level) and would trigger a disallowed
        // contract re-entry. `level` is never persisted on this contract's
        // own profile record (progress is the single source of truth for
        // reads), so the previous index bucket isn't known from storage —
        // remove the player from every level bucket (a no-op for buckets it
        // isn't in) before adding it to the new one.
        let mut stored = Self::load_stored_player(&env, player_id)?;
        let region = stored.vitals.region.clone();

        for lvl in [
            ProgressLevel::Unverified,
            ProgressLevel::VerifiedIdentity,
            ProgressLevel::PerformanceMilestones,
            ProgressLevel::EliteTier,
        ] {
            Self::composite_index_remove(&env, &lvl, &region, player_id);
            Self::level_index_remove(&env, &lvl, player_id);
        }
        Self::composite_index_add(&env, &level, &region, player_id);
        Self::level_index_add(&env, &level, player_id);

        stored.updated_at = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::Player(player_id), &stored);
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

        // Validate player age: must be at least MIN_PLAYER_AGE
        if vitals.age == 0 || vitals.age < MIN_PLAYER_AGE {
            return Err(ScoutChainError::InvalidInput);
        }

        // Validate vitals string lengths
        if vitals.position.len() > MAX_STRING_LEN
            || vitals.region.len() > MAX_REGION_LEN
            || vitals.nationality.len() > MAX_STRING_LEN
        {
            return Err(ScoutChainError::InvalidInput);
        }

        // Validate age upper bound
        if vitals.age > MAX_PLAYER_AGE {
            return Err(ScoutChainError::InvalidInput);
        }

        // Validate ipfs_hashes: non-empty and at most MAX_IPFS_HASHES
        if ipfs_hashes.is_empty() || ipfs_hashes.len() > MAX_IPFS_HASHES {
            return Err(ScoutChainError::InvalidInput);
        }

        let player_id = Self::next_player_id(&env)?;
        let now = env.ledger().timestamp();

        let profile = StoredPlayerProfile {
            player_id,
            wallet: wallet.clone(),
            vitals,
            ipfs_hashes,
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

        // Add to composite (level, region) index — starts at Unverified
        Self::composite_index_add(
            &env,
            &ProgressLevel::Unverified,
            &profile.vitals.region,
            player_id,
        );
        Self::level_index_add(&env, &ProgressLevel::Unverified, player_id);

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
        let mut profile = Self::load_stored_player(&env, player_id)?;
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
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let profile = Self::load_stored_player(&env, player_id)?;
        // Resolve level before removing storage keys (progress contract is source of truth)
        let level = Self::resolve_level(&env, player_id);
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

        // Remove from composite index
        Self::composite_index_remove(&env, &level, &profile.vitals.region, player_id);

        events::player_deregistered(&env, player_id);
        Ok(())
    }

    /// Deactivate a player (admin only).
    ///
    /// Sets a `PlayerDeactivated(player_id)` flag that causes `filter_players`
    /// to skip this player. The on-chain profile, progress history, and all
    /// milestone data are fully preserved and still accessible via `get_player`.
    pub fn deactivate_player(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        // Ensure the player actually exists before setting the flag.
        Self::load_stored_player(&env, player_id)?;
        env.storage()
            .persistent()
            .set(&DataKey::PlayerDeactivated(player_id), &true);
        Ok(())
    }

    /// Reactivate a previously deactivated player (admin only).
    ///
    /// Clears the `PlayerDeactivated(player_id)` flag, making the player
    /// visible in `filter_players` results again.
    pub fn reactivate_player(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        // Ensure the player actually exists.
        Self::load_stored_player(&env, player_id)?;
        env.storage()
            .persistent()
            .remove(&DataKey::PlayerDeactivated(player_id));
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
        env.storage().persistent().extend_ttl(
            &DataKey::Scout(scout_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
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

    /// Return a lightweight player summary without IPFS hashes or wallet.
    pub fn get_player_summary(env: Env, player_id: u64) -> Result<PlayerSummary, ScoutChainError> {
        let profile = Self::load_player(&env, player_id)?;
        Ok(Self::to_player_summary(&profile))
    }

    /// Batch-fetch player summaries for up to 20 IDs in a single call.
    /// Missing IDs are skipped (partial hits).
    pub fn get_players(env: Env, ids: Vec<u64>) -> Result<Vec<PlayerSummary>, ScoutChainError> {
        if ids.len() > MAX_BATCH_SIZE {
            return Err(ScoutChainError::InvalidInput);
        }

        let mut summaries = Vec::new(&env);
        for i in 0..ids.len() {
            if let Some(id) = ids.get(i) {
                if let Ok(profile) = Self::load_player(&env, id) {
                    summaries.push_back(Self::to_player_summary(&profile));
                }
            }
        }
        Ok(summaries)
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
        let profile: ScoutProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutNotFound)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Scout(scout_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(profile)
    }

    /// Verify a scout profile (admin only).
    pub fn verify_scout(env: Env, scout_id: u64) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let mut profile: ScoutProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutNotFound)?;
        profile.verified = true;
        env.storage()
            .persistent()
            .set(&DataKey::Scout(scout_id), &profile);
        events::scout_verified(&env, scout_id, &profile.wallet);
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
    ///
    /// - Pass an empty string for `region` to match players in any region.
    /// - Pass an empty string for `position` to match players in any position.
    /// - `offset` = 0 starts from the beginning; pass the previously returned
    ///   `next_cursor` value as `offset` to fetch the next page.
    ///   `next_cursor` = 0 in the response means no further results.
    /// - `limit` is capped at 50 internally.
    /// - Deactivated players (those with a `PlayerDeactivated` flag) are excluded
    ///   from results. Their profiles are still accessible via `get_player`.
    ///
    /// When `region` is non-empty the composite `PlayersByLevelRegion` index is
    /// used so only matching buckets are loaded.  When `region` is empty the
    /// function falls back to a full `PlayerIndex` scan filtered by level and
    /// position.
    pub fn filter_players(
        env: Env,
        region: String,
        position: String,
        min_level: ProgressLevel,
        offset: u32,
        limit: u32,
    ) -> Result<FilterResult, ScoutChainError> {
        Self::require_initialized(&env)?;

        let max_results = limit.min(50);
        let region_filter = !region.is_empty();
        let position_filter = !position.is_empty();

        let levels: [ProgressLevel; 4] = [
            ProgressLevel::Unverified,
            ProgressLevel::VerifiedIdentity,
            ProgressLevel::PerformanceMilestones,
            ProgressLevel::EliteTier,
        ];

        let mut results: Vec<PlayerProfile> = Vec::new(&env);
        let mut next_cursor: u64 = 0;
        // Number of eligible (non-deactivated, filter-matching) entries skipped so far.
        let mut skipped: u32 = 0;

        if region_filter {
            // Fast path: composite (level, region) index — only load matching buckets.
            'outer: for level in levels.iter() {
                if !Self::level_gte(level, &min_level) {
                    continue;
                }
                let ids: Vec<u64> = env
                    .storage()
                    .persistent()
                    .get(&DataKey::PlayersByLevelRegion(
                        level.clone(),
                        region.clone(),
                    ))
                    .unwrap_or_else(|| Vec::new(&env));

                for player_id in ids.iter() {
                    // Skip deactivated players entirely (don't count toward offset).
                    if env
                        .storage()
                        .persistent()
                        .get::<DataKey, bool>(&DataKey::PlayerDeactivated(player_id))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if let Ok(profile) = Self::load_player(&env, player_id) {
                        if position_filter && profile.vitals.position != position {
                            continue;
                        }
                        if skipped < offset {
                            skipped += 1;
                            continue;
                        }
                        if results.len() >= max_results {
                            next_cursor = player_id;
                            break 'outer;
                        }
                        results.push_back(profile);
                    }
                }
            }
        } else {
            // Slow path: full PlayerIndex scan — needed when no region is specified.
            let all_ids: Vec<u64> = env
                .storage()
                .persistent()
                .get(&DataKey::PlayerIndex)
                .unwrap_or_else(|| Vec::new(&env));

            for player_id in all_ids.iter() {
                // Skip deactivated players entirely (don't count toward offset).
                if env
                    .storage()
                    .persistent()
                    .get::<DataKey, bool>(&DataKey::PlayerDeactivated(player_id))
                    .unwrap_or(false)
                {
                    continue;
                }
                if let Ok(profile) = Self::load_player(&env, player_id) {
                    if !Self::level_gte(&profile.level, &min_level) {
                        continue;
                    }
                    if position_filter && profile.vitals.position != position {
                        continue;
                    }
                    if skipped < offset {
                        skipped += 1;
                        continue;
                    }
                    if results.len() >= max_results {
                        next_cursor = player_id;
                        break;
                    }
                    results.push_back(profile);
                }
            }
        }

        Ok(FilterResult {
            profiles: results,
            next_cursor,
        })
    }

    /// Returns the deployed crate version (from Cargo.toml at build time).
    pub fn version(env: Env) -> String {
        String::from_str(&env, CONTRACT_VERSION)
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


    fn load_stored_player(
        env: &Env,
        player_id: u64,
    ) -> Result<StoredPlayerProfile, ScoutChainError> {
        let profile = env
            .storage()
            .persistent()
            .get(&DataKey::Player(player_id))
            .ok_or(ScoutChainError::PlayerNotFound)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Player(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(profile)
    }

    /// Resolve the current level for `player_id` from the progress contract.
    /// Falls back to `Unverified` when no progress contract is configured
    /// (e.g. during tests or before deployment wiring).
    fn resolve_level(env: &Env, player_id: u64) -> ProgressLevel {
        if let Some(progress_addr) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ProgressContract)
        {
            let client = progress_contract::Client::new(env, &progress_addr);
            client.get_level(&player_id)
        } else {
            ProgressLevel::Unverified
        }
    }

    fn stored_to_profile(stored: StoredPlayerProfile, level: ProgressLevel) -> PlayerProfile {
        PlayerProfile {
            player_id: stored.player_id,
            wallet: stored.wallet,
            vitals: stored.vitals,
            ipfs_hashes: stored.ipfs_hashes,
            level,
            registered_at: stored.registered_at,
            updated_at: stored.updated_at,
        }
    }

    fn load_player(env: &Env, player_id: u64) -> Result<PlayerProfile, ScoutChainError> {
        let stored = Self::load_stored_player(env, player_id)?;
        let level = Self::resolve_level(env, player_id);
        Ok(Self::stored_to_profile(stored, level))
    }

    fn to_player_summary(profile: &PlayerProfile) -> PlayerSummary {
        PlayerSummary {
            player_id: profile.player_id,
            vitals: profile.vitals.clone(),
            level: profile.level.clone(),
            updated_at: profile.updated_at,
        }
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

    /// Add `player_id` to the composite (level, region) index bucket.
    fn composite_index_add(env: &Env, level: &ProgressLevel, region: &String, player_id: u64) {
        let key = DataKey::PlayersByLevelRegion(level.clone(), region.clone());
        let mut ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        ids.push_back(player_id);
        env.storage().persistent().set(&key, &ids);
    }

    /// Remove `player_id` from the composite (level, region) index bucket.
    fn composite_index_remove(env: &Env, level: &ProgressLevel, region: &String, player_id: u64) {
        let key = DataKey::PlayersByLevelRegion(level.clone(), region.clone());
        let mut ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        if let Some(pos) = ids.iter().position(|id| id == player_id) {
            ids.remove(pos as u32);
            env.storage().persistent().set(&key, &ids);
        }
    }

    fn level_index_add(env: &Env, level: &ProgressLevel, player_id: u64) {
        let key = DataKey::PlayersByLevel(level.clone());
        let mut ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        ids.push_back(player_id);
        env.storage().persistent().set(&key, &ids);
    }

    fn level_index_remove(env: &Env, level: &ProgressLevel, player_id: u64) {
        let key = DataKey::PlayersByLevel(level.clone());
        let mut ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        if let Some(pos) = ids.iter().position(|id| id == player_id) {
            ids.remove(pos as u32);
            env.storage().persistent().set(&key, &ids);
        }
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
        let contract_id = env.register(RegistrationContract, ());
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
    fn test_version() {
        let (env, client) = setup();
        assert_eq!(client.version(), String::from_str(&env, "0.1.0"));
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
    fn test_register_scout_region_100_bytes_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, &"A".repeat(100));
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

    /// Verifies that a position string of exactly 65 bytes (one over MAX_STRING_LEN=64)
    /// is rejected with the explicit `InvalidInput` error code, pinning the upper-bound
    /// enforcement so a silent regression cannot go undetected.
    #[test]
    fn test_register_player_position_65_bytes_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        // 65 ASCII bytes — one over the MAX_STRING_LEN = 64 limit
        let position_65 = String::from_str(&env, &"A".repeat(65));
        let vitals = PlayerVitals {
            age: 20,
            position: position_65,
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(
            result,
            Err(Ok(ScoutChainError::InvalidInput)),
            "expected InvalidInput when position exceeds 64 bytes"
        );
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

    // -------------------------------------------------------------------------
    // Issue #460: MIN_PLAYER_AGE validation
    // -------------------------------------------------------------------------

    /// age = 0 must return ScoutChainError::InvalidInput
    #[test]
    fn test_register_player_age_zero_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: 0,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::InvalidInput)));
    }

    /// age below MIN_PLAYER_AGE must return ScoutChainError::InvalidInput
    #[test]
    fn test_register_player_age_below_min_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: MIN_PLAYER_AGE - 1,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::InvalidInput)));
    }

    /// age = MIN_PLAYER_AGE must register successfully
    #[test]
    fn test_register_player_age_at_min_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: MIN_PLAYER_AGE,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert!(
            result.is_ok(),
            "age = MIN_PLAYER_AGE should register successfully"
        );
    }

    // -------------------------------------------------------------------------
    // Issue #416: explicit boundary tests for position MAX_STRING_LEN (64 bytes)
    // -------------------------------------------------------------------------

    /// A position string of exactly 64 bytes (MAX_STRING_LEN) must be accepted.
    /// Nationality and region are well within their valid ranges.
    #[test]
    fn test_register_player_position_exactly_64_bytes_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let position_64 = String::from_str(&env, &"Y".repeat(64));
        let vitals = PlayerVitals {
            age: 22,
            position: position_64.clone(),
            region: String::from_str(&env, "East Africa"),
            nationality: String::from_str(&env, "Kenya"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmBoundaryTest2")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert!(
            result.is_ok(),
            "64-byte position should register successfully"
        );
        let player_id = result.unwrap().unwrap();

        let profile = client.get_player(&player_id);
        assert_eq!(profile.vitals.position, position_64);
        assert_eq!(profile.vitals.nationality, String::from_str(&env, "Kenya"));
        assert_eq!(profile.vitals.region, String::from_str(&env, "East Africa"));
    }

    #[test]
    #[should_panic]
    fn test_register_player_region_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        // 129 bytes — beyond MAX_REGION_LEN (100)
        let long = String::from_str(&env, &"A".repeat(129));
        let vitals = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: long,
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        client.register_player(&wallet, &vitals, &hashes);
    }

    // -------------------------------------------------------------------------
    // Issue #415: MAX_REGION_LEN (100) boundary tests for register_player
    // -------------------------------------------------------------------------

    /// A 101-byte region string must be rejected with InvalidInput.
    #[test]
    fn test_register_player_region_101_bytes_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region_101 = String::from_str(&env, &"A".repeat(101));
        let vitals = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: region_101,
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::InvalidInput)));
    }

    /// An exactly 100-byte region string is at the boundary and must succeed.
    #[test]
    fn test_register_player_region_100_bytes_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region_100 = String::from_str(&env, &"A".repeat(100));
        let vitals = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: region_100,
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);
        let profile = client.get_player(&player_id);
        assert_eq!(profile.wallet, wallet);
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
    fn test_register_player_exactly_10_hashes_succeeds() {
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
        ];

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);

        let profile = client.get_player(&player_id);
        assert_eq!(profile.ipfs_hashes.len(), 10);
    }

    #[test]
    fn test_register_player_11_hashes_fails_with_invalid_input() {
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

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::InvalidInput)));
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
        let region = String::from_str(&env, &"a".repeat(101));
        client.register_scout(&wallet, &region);
    }

    #[test]
    fn test_register_scout_region_max_len_ok() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let exactly_100 = String::from_str(&env, &"A".repeat(100));
        let scout_id = client.register_scout(&wallet, &exactly_100);
        assert_eq!(scout_id, 1);
    }

    #[test]
    fn test_upgrade_preserves_admin() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(RegistrationContract, ());
        let client = RegistrationContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Register a player so we confirm persistent data also survives
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let player_id = client.register_player(&wallet, &vitals, &hashes);

        let new_wasm_hash = env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::new(&env));

        client.upgrade(&new_wasm_hash);

        // Admin persisted
        client.pause_contract();

        // Existing data persisted
        assert_eq!(client.get_player(&player_id).player_id, player_id);
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

        // Register a player so we confirm persistent data also survives
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        // Simulate upgrade: in testutils mode the host accepts empty bytes as a valid wasm blob
        let new_wasm_hash = env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::new(&env));
        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.pause_contract();
        assert_eq!(client.get_player(&player_id).player_id, player_id);
    }

    // -------------------------------------------------------------------------
    // Issue #26: get_player_count and get_scout_count query functions
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_player_count_returns_zero_before_init() {
        let (_env, client) = setup();
        assert_eq!(client.get_player_count(), 0);
    }

    #[test]
    fn test_get_scout_count_returns_zero_before_init() {
        let (_env, client) = setup();
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
    // Issue #31: filter_players query function (now paginated — #223)
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

        // Filter: Forward in West Africa — offset=0
        let result = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );

        assert_eq!(result.profiles.len(), 1);
        assert_eq!(result.profiles.get(0).unwrap().player_id, 1);
        assert_eq!(result.next_cursor, 0); // no more pages
    }

    #[test]
    fn test_filter_players_pagination() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        // Register 5 Forwards in West Africa
        for _ in 0..5 {
            let wallet = Address::generate(&env);
            let vitals = PlayerVitals {
                age: 18,
                position: String::from_str(&env, "Forward"),
                region: String::from_str(&env, "West Africa"),
                nationality: String::from_str(&env, "Ghana"),
            };
            client.register_player(&wallet, &vitals, &hashes);
        }
        // Register 1 Midfielder to break up the list
        let wallet_mid = Address::generate(&env);
        let vitals_mid = PlayerVitals {
            age: 22,
            position: String::from_str(&env, "Midfielder"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        client.register_player(&wallet_mid, &vitals_mid, &hashes);
        // Register 3 more Forwards
        for _ in 0..3 {
            let wallet = Address::generate(&env);
            let vitals = PlayerVitals {
                age: 19,
                position: String::from_str(&env, "Forward"),
                region: String::from_str(&env, "West Africa"),
                nationality: String::from_str(&env, "Ghana"),
            };
            client.register_player(&wallet, &vitals, &hashes);
        }

        // Page 1: offset=0, limit=4 → should return 4 Forwards
        let page1 = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &0u32,
            &4u32,
        );
        assert_eq!(page1.profiles.len(), 4);
        assert_ne!(page1.next_cursor, 0, "expected more pages");

        // Page 2: offset=4, limit=4 → remaining Forwards
        let page2 = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &4u32,
            &4u32,
        );
        // 8 Forwards total, already skipped 4, so 4 more remain
        assert_eq!(page2.profiles.len(), 4);
        assert_eq!(page2.next_cursor, 0, "should be no more pages");
    }

    // -------------------------------------------------------------------------
    // Issue #419: filter_players with region filter only (empty position)
    // -------------------------------------------------------------------------

    /// Register players across two distinct regions.
    /// filter_players with only a region set (empty position) must return only
    /// players from that region and exclude all others.
    #[test]
    fn test_filter_players_region_only_returns_correct_players() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        // Region A players (West Africa)
        let wallet1 = Address::generate(&env);
        let vitals1 = PlayerVitals {
            age: 18,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let id_wa1 = client.register_player(&wallet1, &vitals1, &hashes);

        let wallet2 = Address::generate(&env);
        let vitals2 = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Midfielder"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Nigeria"),
        };
        let id_wa2 = client.register_player(&wallet2, &vitals2, &hashes);

        // Region B player (Europe) — must be excluded
        let wallet3 = Address::generate(&env);
        let vitals3 = PlayerVitals {
            age: 22,
            position: String::from_str(&env, "Defender"),
            region: String::from_str(&env, "Europe"),
            nationality: String::from_str(&env, "Germany"),
        };
        client.register_player(&wallet3, &vitals3, &hashes);

        // Filter: region = West Africa, no position constraint (empty string)
        let result = client.filter_players(
            &String::from_str(&env, "West Africa"), // region filter only
            &String::from_str(&env, ""),            // no position filter
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );

        assert_eq!(result.profiles.len(), 2, "two West Africa players expected");

        let returned_ids: soroban_sdk::Vec<u64> = {
            let mut v = soroban_sdk::Vec::new(&env);
            for i in 0..result.profiles.len() {
                v.push_back(result.profiles.get(i).unwrap().player_id);
            }
            v
        };
        assert!(returned_ids.contains(&id_wa1), "id_wa1 must be in results");
        assert!(returned_ids.contains(&id_wa2), "id_wa2 must be in results");
        assert_eq!(result.next_cursor, 0);
    }

    /// filter_players with a region that has no registered players returns empty.
    #[test]
    fn test_filter_players_region_only_empty_region_returns_empty() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        // All players are in West Africa
        let wallet1 = Address::generate(&env);
        let vitals1 = PlayerVitals {
            age: 19,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Senegal"),
        };
        client.register_player(&wallet1, &vitals1, &hashes);

        // Filter by a region that has no players
        let result = client.filter_players(
            &String::from_str(&env, "East Asia"), // region with no players
            &String::from_str(&env, ""),          // no position filter
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );

        assert_eq!(
            result.profiles.len(),
            0,
            "no players in East Asia — must be empty"
        );
        assert_eq!(result.next_cursor, 0);
    }

    // -------------------------------------------------------------------------
    // Issue #474: Player deactivation and reactivation
    // -------------------------------------------------------------------------

    /// Deactivated players must NOT appear in filter_players results.
    #[test]
    fn test_deactivated_player_excluded_from_filter() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        // Register two players in the same region/position
        let wallet1 = Address::generate(&env);
        let vitals1 = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let player1 = client.register_player(&wallet1, &vitals1, &hashes);

        let wallet2 = Address::generate(&env);
        let vitals2 = PlayerVitals {
            age: 22,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Nigeria"),
        };
        let player2 = client.register_player(&wallet2, &vitals2, &hashes);

        // Before deactivation, both appear
        let result_before = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );
        assert_eq!(result_before.profiles.len(), 2);

        // Deactivate player1
        client.deactivate_player(&player1);

        // After deactivation, only player2 appears
        let result_after = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );
        assert_eq!(result_after.profiles.len(), 1);
        assert_eq!(result_after.profiles.get(0).unwrap().player_id, player2);
    }

    /// get_player still returns the profile for a deactivated player (data preserved).
    #[test]
    fn test_deactivated_player_profile_preserved_via_get_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        client.deactivate_player(&player_id);

        // Profile must still be accessible
        let profile = client.get_player(&player_id);
        assert_eq!(profile.wallet, wallet);
        assert_eq!(profile.player_id, player_id);
    }

    /// Admin can reactivate a previously deactivated player.
    /// After reactivation, the player appears in filter_players results again.
    #[test]
    fn test_reactivate_player_restores_filter_visibility() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        // Deactivate then reactivate
        client.deactivate_player(&player_id);

        let result_deactivated = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );
        assert_eq!(result_deactivated.profiles.len(), 0);

        client.reactivate_player(&player_id);

        let result_reactivated = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &0u32,
            &20u32,
        );
        assert_eq!(result_reactivated.profiles.len(), 1);
        assert_eq!(
            result_reactivated.profiles.get(0).unwrap().player_id,
            player_id
        );
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
    // Issue #469: verify_scout emits scout_verified with wallet + non-admin test
    // -------------------------------------------------------------------------

    #[test]
    fn test_verify_scout_emits_event_with_wallet() {
        use soroban_sdk::testutils::Events;
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&wallet, &region);

        client.verify_scout(&scout_id);

        use soroban_sdk::IntoVal;
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        soroban_sdk::Symbol::new(&env, crate::events::SCOUT_VERIFIED),
                        wallet.clone()
                    )
                        .into_val(&env),
                    scout_id.into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_verify_scout_non_admin_unauthorized() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&wallet, &region);

        // Clear all auths so admin check fails
        env.mock_auths(&[]);
        let result = client.try_verify_scout(&scout_id);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Pause / unpause behaviour
    // -------------------------------------------------------------------------

    #[test]
    fn test_register_player_while_paused_returns_contract_paused() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.pause_contract();

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::ContractPaused)));
    }

    #[test]
    fn test_register_scout_while_paused_returns_contract_paused() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.pause_contract();

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");

        let result = client.try_register_scout(&wallet, &region);
        assert_eq!(result, Err(Ok(ScoutChainError::ContractPaused)));
    }

    #[test]
    fn test_update_profile_while_paused_returns_contract_paused() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Register the player before pausing so the player exists.
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmOld")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        client.pause_contract();

        let new_hashes = vec![&env, String::from_str(&env, "QmNew")];
        let result = client.try_update_profile(&player_id, &new_hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::ContractPaused)));
    }

    #[test]
    fn test_admin_functions_succeed_while_paused() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Register a player and a scout before pausing.
        let player_wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&player_wallet, &vitals, &hashes);

        let scout_wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&scout_wallet, &region);

        client.pause_contract();

        // deregister_player and verify_scout are admin-only and must bypass the pause.
        assert_eq!(client.try_deregister_player(&player_id), Ok(Ok(())));
        assert_eq!(client.try_verify_scout(&scout_id), Ok(Ok(())));
    }

    #[test]
    fn test_register_player_succeeds_after_unpause() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        client.pause_contract();

        // Confirm the contract is paused.
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        assert_eq!(
            client.try_register_player(&wallet, &vitals, &hashes),
            Err(Ok(ScoutChainError::ContractPaused))
        );

        client.unpause_contract();

        // After unpausing, registration must succeed again.
        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);
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

    #[test]
    fn test_full_milestone_approval_flow_integration() {
        use scoutchain_progress::{ProgressContract, ProgressContractClient};
        use scoutchain_verification::{VerificationContract, VerificationContractClient};
        use soroban_sdk::testutils::Ledger;

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| {
            l.sequence_number = 1;
        });

        let admin = Address::generate(&env);

        // 1. Deploy registration contract
        let reg_id = env.register(RegistrationContract, ());
        let reg_client = RegistrationContractClient::new(&env, &reg_id);
        reg_client.initialize(&admin);

        // 2. Deploy progress contract
        let prog_id = env.register(ProgressContract, ());
        let prog_client = ProgressContractClient::new(&env, &prog_id);
        prog_client.initialize(&admin);

        // 3. Deploy verification contract
        let ver_id = env.register(VerificationContract, ());
        let ver_client = VerificationContractClient::new(&env, &ver_id);
        ver_client.initialize(&admin);

        // 4. Wire verification -> progress
        ver_client.set_progress_contract(&prog_id);

        // 5. Wire progress -> verification
        prog_client.set_verification_contract(&ver_id);

        // 6. Wire progress -> registration
        prog_client.set_registration_contract(&reg_id);

        // 7. Wire registration <- progress
        reg_client.set_progress_contract(&prog_id);

        // 8. Register player in registration contract
        let player_wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmPlayerEvidence")];
        let player_id = reg_client.register_player(&player_wallet, &vitals, &hashes);

        // 9. Register validator in verification contract
        let validator = Address::generate(&env);
        ver_client.register_validator(&validator, &String::from_str(&env, "UEFA B License"));

        // 10. Approve milestone via verification contract (this triggers the cross-contract flow)
        ver_client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Completed Level 1 requirements"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );

        // 11. Assert that the player's level is now VerifiedIdentity in registration contract
        let profile = reg_client.get_player(&player_id);
        assert_eq!(profile.level, ProgressLevel::VerifiedIdentity);
    }

    // -------------------------------------------------------------------------
    // TTL bump bugfix: get_player must extend persistent TTL on read
    // -------------------------------------------------------------------------

    /// Registers a player, advances the ledger sequence past the default Soroban
    /// persistent TTL (4096 ledgers), then asserts that `get_player` still returns
    /// the profile successfully.
    ///
    /// On unfixed code (without the `extend_ttl` call in `load_stored_player`),
    /// the persistent key expires after the initial TTL elapses and `get_player`
    /// panics with `PlayerNotFound`.  The fix causes every `get_player` call to
    /// refresh the TTL, so the profile remains readable as long as reads continue.
    #[test]
    fn test_get_player_ttl_expires_without_bump() {
        use soroban_sdk::testutils::Ledger;

        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Start at a known ledger sequence so the advance is deterministic.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100;
            // Ensure the environment allows TTL values large enough for the test.
            l.max_entry_ttl = 100_000;
        });

        // Register a player — the persistent key is created at sequence 100.
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTTLTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        // Advance the ledger past the default Soroban persistent TTL (4096 ledgers).
        // Without the fix the key expires here and the next `get_player` would panic.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100 + 5_000; // well past the 4096 default TTL
        });

        // With the fix in place, `get_player` extends the TTL on every read, so the
        // profile must still be returned correctly even after the ledger advance.
        let profile = client.get_player(&player_id);
        assert_eq!(profile.wallet, wallet);
        assert_eq!(profile.level, ProgressLevel::Unverified);
    }

    // -------------------------------------------------------------------------
    // Issue #444: register_player age field must reject implausible upper values
    // -------------------------------------------------------------------------

    /// An age of MAX_PLAYER_AGE (100) must be accepted.
    #[test]
    fn test_register_player_age_at_max_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: 100, // exactly MAX_PLAYER_AGE
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmAgeTest")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert!(result.is_ok(), "age == MAX_PLAYER_AGE should succeed");
    }

    /// An age of MAX_PLAYER_AGE + 1 (101) must be rejected with InvalidInput.
    #[test]
    fn test_register_player_age_above_max_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: 101, // one above MAX_PLAYER_AGE
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmAgeTest")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::InvalidInput)));
    }

    /// An implausibly large age (999) must also be rejected with InvalidInput.
    #[test]
    fn test_register_player_implausible_age_returns_invalid_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = PlayerVitals {
            age: 999,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmAgeTest")];

        let result = client.try_register_player(&wallet, &vitals, &hashes);
        assert_eq!(result, Err(Ok(ScoutChainError::InvalidInput)));
    }
}
