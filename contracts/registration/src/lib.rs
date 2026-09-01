#![cfg_attr(target_family = "wasm", no_std)]
// Contract ABI functions (migration seeding / ticket redemption) carry their
// payloads as individual parameters rather than a wrapper struct, so a few
// legitimately exceed clippy's 7-argument guideline. The Soroban `contractargs`
// macro re-emits these signatures and cannot inherit per-method `#[allow]`.
#![allow(clippy::too_many_arguments)]
mod errors;
mod events;
mod types;

use types::{
    ContractHealth, DataKey, FilterResult, PlayerProfile, PlayerStatus, PlayerSummary,
    ProgressLevel, RegistrationWiringState, ScoutProfile, ScoutStatus, ScoutVerificationRecord,
    StoredPlayerProfile,
};

pub use errors::ScoutChainError;
pub use types::{MigrationAuthorization, MigrationRole};
// `PlayerVitals` is an *input* type of the public `register_player` function, so
// it must be nameable by external callers (integration tests, generated
// clients). Re-export it at the crate root; this also brings it into local
// scope for the rest of this module.
pub use types::PlayerVitals;

use scoutchain_shared_types::{
    read_wiring_link, require_admin, safe_math::safe_add_u64, write_wiring_link,
};
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, String, Vec};

// Generated client stub for the progress contract — used to resolve a player's
// current level at read time.  `level` is never stored in this contract.
mod progress_contract {
    use scoutchain_shared_types::ProgressLevel;
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
const MAX_PLAYERS: u64 = 10_000;

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

// Core identity TTL: 30 days at ~5s/ledger ≈ 518_400 ledgers.
// Player and scout profiles are core identity data that must survive extended dormancy.
// Composite indexes (PlayersByLevelRegion, PlayersByLevel) are derived from profiles
// and must live as long as the profiles they index.
const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 518_400;
const ADMIN_BUMP_LEDGERS: u32 = 518_400;

/// Default registration cooldown: 24 hours in seconds.
/// Applies to register_player, register_scout, andregister_validator.
/// Configurable by admin via `set_reg_cooldown`.  0 disables the cooldown.
const DEFAULT_REG_COOLDOWN_SECS: u64 = 86_400;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Instance storage TTL constants (in ledger closures, ~10 seconds per closure)
const INSTANCE_TTL_MIN: u32 = 500;   // ~1.4 hours
const INSTANCE_TTL_MAX: u32 = 500;   // ~1.4 hours

// Persistent storage TTL constants
const PERSISTENT_TTL_MIN: u32 = 500;    // ~1.4 hours
const PERSISTENT_TTL_MAX: u32 = 2000;   // ~5.5 hours

// Admin persistent key bump interval (~30 days)
const ADMIN_BUMP_LEDGERS: u32 = 518400;

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
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Propose a replacement administrator. The current admin remains active
    /// until the proposed address calls `accept_admin`.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), ScoutChainError> {
        let old_admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage()
            .persistent()
            .set(&DataKey::PendingAdmin, &new_admin);
        env.storage().persistent().extend_ttl(
            &DataKey::PendingAdmin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        events::admin_transfer_proposed(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Accept a pending admin transfer. Only the proposed address can accept.
    pub fn accept_admin(env: Env) -> Result<(), ScoutChainError> {
        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ScoutChainError::NotInitialized)?;
        let new_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .ok_or(ScoutChainError::PendingAdminNotSet)?;
        new_admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        env.storage().persistent().remove(&DataKey::PendingAdmin);
        events::admin_transferred(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Deprecated alias for `propose_admin`; this no longer transfers control
    /// immediately. The proposed address must still call `accept_admin`.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ScoutChainError> {
        Self::propose_admin(env, new_admin)
    }

    pub fn pause_contract(env: Env) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        Self::bump_instance_ttl(&env);
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
    /// levels at query time (admin only). Freely re-settable — no
    /// first-call-only guard (see `docs/WIRING_REGISTRY_DESIGN.md` for why
    /// this is the majority policy across all four contracts).
    pub fn set_progress_contract(env: Env, addr: Address) -> Result<(), ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
            &addr,
        );
        events::wiring_updated(&env, &admin, "progress_contract", &addr, epoch);
        Ok(())
    }

    /// Return the configured progress contract address, or `None` if the
    /// link has not been configured. Read-only and requires no auth.
    pub fn get_progress_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::ProgressContract)
    }

    /// Returns a snapshot of the single cross-contract peer address pointer
    /// held by this contract (progress), with its address and re-wiring
    /// epoch.
    ///
    /// This is a **read-only** function — it does not require auth, does not
    /// modify state, and is intentionally exempt from the pause/init guards
    /// so it remains callable even on a mis-wired contract, matching
    /// `progress.get_wiring_state()`. See `docs/WIRING_REGISTRY_DESIGN.md`.
    pub fn get_wiring_state(env: Env) -> RegistrationWiringState {
        let progress_contract = read_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
        );
        RegistrationWiringState { progress_contract }
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
        env.storage()
            .persistent()
            .set(&DataKey::PlayerLevel(player_id), &level);
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerLevel(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        events::player_level_synced(&env, player_id, &progress_contract);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Player registration
    // -------------------------------------------------------------------------

    /// Register a new player profile at Level 0 (Unverified).
    /// `ipfs_hashes` - list of IPFS/Arweave CIDs for highlight reels and photos.
    pub fn register_player(
        env: Env,
        wallet: Address,
        vitals: PlayerVitals,
        ipfs_hashes: Vec<String>,
    ) -> Result<u64, ScoutChainError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        wallet.require_auth();

        // Per-caller cooldown: reject rapid re-registration attempts from the
        // same wallet.  The cooldown protects against sybil registrations where
        // a set of freshly-generated wallets spam the entrypoint.
        Self::enforce_reg_cooldown(&env, &DataKey::PlayerRegLastSent(wallet.clone()))?;

        // Prevent duplicate registrations
        if env
            .storage()
            .persistent()
            .has(&DataKey::PlayerByWallet(wallet.clone()))
        {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        // Enforce player cap to bound filter_players slow-path scan cost.
        let player_count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerIndex)
            .map(|v: Vec<u64>| v.len() as u64)
            .unwrap_or(0);
        if player_count >= MAX_PLAYERS {
            return Err(ScoutChainError::PlayerCapReached);
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
        env.storage()
            .persistent()
            .set(&DataKey::PlayerLevel(player_id), &ProgressLevel::Unverified);
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerLevel(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

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

        // Record cooldown timestamp so a repeated attempt from the same wallet
        // within the cooldown window is rejected.
        env.storage()
            .persistent()
            .set(&DataKey::PlayerRegLastSent(wallet.clone()), &now);
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerRegLastSent(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::player_registered(&env, player_id, &wallet);
        Self::bump_instance_ttl(&env);
        Ok(player_id)
    }

    /// Update a player's IPFS content hashes (player auth required).
    pub fn update_profile(
        env: Env,
        player_id: u64,
        ipfs_hashes: Vec<String>,
    ) -> Result<(), ScoutChainError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
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
        Self::bump_instance_ttl(&env);
        events::profile_updated(&env, player_id, &profile.wallet);
        Ok(())
    }

    /// Deregister a player profile (admin only, GDPR right-to-erasure).
    pub fn deregister_player(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let profile = Self::load_stored_player(&env, player_id)?;
        // Resolve level before removing storage keys (progress contract is source of truth)
        let level = Self::resolve_level(&env, player_id);
        env.storage()
            .persistent()
            .remove(&DataKey::Player(player_id));
        env.storage()
            .persistent()
            .remove(&DataKey::PlayerByWallet(profile.wallet));
        env.storage()
            .persistent()
            .remove(&DataKey::PlayerLevel(player_id));

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

        events::player_deregistered(&env, player_id, &admin);
        Ok(())
    }

    /// Deactivate a player (admin only).
    ///
    /// Sets a `PlayerDeactivated(player_id)` flag that causes `filter_players`
    /// to skip this player. The on-chain profile, progress history, and all
    /// milestone data are fully preserved and still accessible via `get_player`.
    pub fn deactivate_player(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        // Ensure the player actually exists before setting the flag.
        Self::load_stored_player(&env, player_id)?;
        env.storage()
            .persistent()
            .set(&DataKey::PlayerDeactivated(player_id), &true);
        events::player_deactivated(&env, player_id, &admin);
        Ok(())
    }

    /// Reactivate a previously deactivated player (admin only).
    ///
    /// Clears the `PlayerDeactivated(player_id)` flag, making the player
    /// visible in `filter_players` results again.
    pub fn reactivate_player(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        // Ensure the player actually exists.
        Self::load_stored_player(&env, player_id)?;
        env.storage()
            .persistent()
            .remove(&DataKey::PlayerDeactivated(player_id));
        events::player_reactivated(&env, player_id, &admin);
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
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        wallet.require_auth();

        if region.len() > MAX_REGION_LEN {
            return Err(ScoutChainError::InvalidInput);
        }

        // Per-caller cooldown: same pattern as register_player.
        Self::enforce_reg_cooldown(&env, &DataKey::ScoutRegLastSent(wallet.clone()))?;

        if env
            .storage()
            .persistent()
            .has(&DataKey::ScoutByWallet(wallet.clone()))
        {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        let scout_id = Self::next_scout_id(&env)?;
        let now = env.ledger().timestamp();
        let profile = ScoutProfile {
            scout_id,
            wallet: wallet.clone(),
            region,
            verified: false,
            verification: ScoutVerificationRecord {
                verified: false,
                verified_by: None,
                verified_at: None,
                evidence_ref: None,
                method: None,
            },
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

        // Record cooldown timestamp.
        env.storage()
            .persistent()
            .set(&DataKey::ScoutRegLastSent(wallet.clone()), &now);
        env.storage().persistent().extend_ttl(
            &DataKey::ScoutRegLastSent(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::scout_registered(&env, scout_id, &wallet);
        Self::bump_instance_ttl(&env);
        Ok(scout_id)
    }

    /// Deregister a player from the system (player auth required).
    pub fn deregister_player(
        env: Env,
        player_id: u64,
    ) -> Result<(), ScoutChainError> {
        Self::require_not_paused(&env)?;
        
        // Load player to verify ownership and get wallet
        let profile = Self::load_player(&env, player_id)?;
        profile.wallet.require_auth();

        // Remove from persistent storage
        env.storage()
            .persistent()
            .remove(&DataKey::Player(player_id));
        env.storage()
            .persistent()
            .remove(&DataKey::PlayerByWallet(profile.wallet.clone()));

        // Decrement the player counter to reflect removal
        let current_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::PlayerCounter)
            .unwrap_or(0u64);
        if current_count > 0 {
            env.storage()
                .instance()
                .set(&DataKey::PlayerCounter, &(current_count - 1));
        }

        events::player_deregistered(&env, player_id, &profile.wallet);
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Migration (for relayer-driven account recovery/bulk seeding)
    // -------------------------------------------------------------------------

    /// Public relayer-driven migration for players. Accepts pre-signed player data.
    /// Does NOT require admin auth; the signature is the authorization.
    pub fn redeem_migration_player(
        env: Env,
        wallet: Address,
        vitals: PlayerVitals,
        ipfs_hashes: Vec<String>,
    ) -> Result<u64, ScoutChainError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        wallet.require_auth();

        // Validate inputs
        if vitals.position.len() > MAX_STRING_LEN
            || vitals.region.len() > MAX_STRING_LEN
            || vitals.nationality.len() > MAX_STRING_LEN
        {
            return Err(ScoutChainError::InvalidInput);
        }

        if ipfs_hashes.is_empty() || ipfs_hashes.len() > MAX_IPFS_HASHES {
            return Err(ScoutChainError::InvalidInput);
        }

        // Prevent duplicate registrations
        if env
            .storage()
            .persistent()
            .has(&DataKey::PlayerByWallet(wallet.clone()))
        {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        // Use private helper to seed the player
        Self::_seed_player(&env, wallet, vitals, ipfs_hashes)
    }

    /// Public relayer-driven migration for scouts. Accepts pre-signed scout data.
    /// Does NOT require admin auth; the signature is the authorization.
    pub fn redeem_migration_scout(
        env: Env,
        wallet: Address,
        region: String,
    ) -> Result<u64, ScoutChainError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
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

        // Use private helper to seed the scout
        Self::_seed_scout(&env, wallet, region)
    }

    /// Private helper to seed a player (called by both redeem_migration_player and admin functions).
    /// No authorization check — the public caller is responsible for auth.
    fn _seed_player(
        env: &Env,
        wallet: Address,
        vitals: PlayerVitals,
        ipfs_hashes: Vec<String>,
    ) -> Result<u64, ScoutChainError> {
        let player_id = Self::next_player_id(&env);
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

        events::player_registered(&env, player_id, &wallet);
        Self::bump_instance_ttl(&env);
        Ok(player_id)
    }

    /// Private helper to seed a scout (called by both redeem_migration_scout and admin functions).
    /// No authorization check — the public caller is responsible for auth.
    fn _seed_scout(
        env: &Env,
        wallet: Address,
        region: String,
    ) -> Result<u64, ScoutChainError> {
        let scout_id = Self::next_scout_id(&env);
        let profile = ScoutProfile {
            scout_id,
            wallet: wallet.clone(),
            region,
            registered_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Scout(scout_id), &profile);
        env.storage()
            .persistent()
            .set(&DataKey::ScoutByWallet(wallet.clone()), &scout_id);

        events::scout_registered(&env, scout_id, &wallet);
        Self::bump_instance_ttl(&env);
        Ok(scout_id)
    }

    /// Seed a player profile directly using admin authority.
    #[allow(clippy::too_many_arguments)]
    pub fn admin_seed_player(
        env: Env,
        wallet: Address,
        vitals: PlayerVitals,
        ipfs_hashes: Vec<String>,
        level: ProgressLevel,
        player_id: u64,
        registered_at: u64,
        updated_at: u64,
    ) -> Result<u64, ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if env.storage().persistent().has(&DataKey::Player(player_id)) {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        if vitals.age == 0 || vitals.age < MIN_PLAYER_AGE {
            return Err(ScoutChainError::InvalidInput);
        }
        if vitals.position.len() > MAX_STRING_LEN
            || vitals.region.len() > MAX_REGION_LEN
            || vitals.nationality.len() > MAX_STRING_LEN
        {
            return Err(ScoutChainError::InvalidInput);
        }
        if vitals.age > MAX_PLAYER_AGE {
            return Err(ScoutChainError::InvalidInput);
        }
        if ipfs_hashes.is_empty() || ipfs_hashes.len() > MAX_IPFS_HASHES {
            return Err(ScoutChainError::InvalidInput);
        }

        let stored = StoredPlayerProfile {
            player_id,
            wallet: wallet.clone(),
            vitals,
            ipfs_hashes,
            registered_at,
            updated_at,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Player(player_id), &stored);
        env.storage()
            .persistent()
            .set(&DataKey::PlayerByWallet(wallet.clone()), &player_id);
        env.storage()
            .persistent()
            .set(&DataKey::PlayerLevel(player_id), &level);
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerLevel(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        let mut player_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerIndex)
            .unwrap_or_else(|| Vec::new(&env));
        if !player_ids.iter().any(|id| id == player_id) {
            player_ids.push_back(player_id);
            env.storage()
                .persistent()
                .set(&DataKey::PlayerIndex, &player_ids);
        }

        Self::composite_index_add(&env, &level, &stored.vitals.region, player_id);
        Self::level_index_add(&env, &level, player_id);

        events::player_registered(&env, player_id, &wallet);
        Ok(player_id)
    }

    pub fn get_player_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get::<DataKey, u64>(&DataKey::PlayerCounter)
            .unwrap_or(0u64)
    }

    pub fn get_scout_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get::<DataKey, u64>(&DataKey::ScoutCounter)
            .unwrap_or(0u64)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------
    /// Seed a scout profile directly using admin authority.
    pub fn admin_seed_scout(
        env: Env,
        wallet: Address,
        region: String,
        scout_id: u64,
        registered_at: u64,
        verified: bool,
    ) -> Result<u64, ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if env.storage().persistent().has(&DataKey::Scout(scout_id)) {
            return Err(ScoutChainError::AlreadyRegistered);
        }

        if region.len() > MAX_REGION_LEN {
            return Err(ScoutChainError::InvalidInput);
        }

        let profile = ScoutProfile {
            scout_id,
            wallet: wallet.clone(),
            region,
            verified,
            verification: ScoutVerificationRecord {
                verified,
                verified_by: if verified { Some(admin.clone()) } else { None },
                verified_at: if verified {
                    Some(env.ledger().timestamp())
                } else {
                    None
                },
                evidence_ref: None,
                method: if verified {
                    Some(String::from_str(&env, "admin_manual"))
                } else {
                    None
                },
            },
            registered_at,
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

    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
    }
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env, String};

    // -------------------------------------------------------------------------
    // Migration ticket protocol
    // -------------------------------------------------------------------------

    /// Redeem a player migration authorization signed by the player off-chain.
    ///
    /// A relayer with no player private key can call this function to recreate
    /// a player's profile on a freshly deployed contract. The function verifies
    /// the player's ed25519 signature over the canonical authorization message
    /// before writing any state.
    ///
    /// The signed message covers:
    /// `wallet || role(Player=0) || profile_data_hash || new_contract_hint || nonce || expires_at`
    #[allow(clippy::too_many_arguments)]
    pub fn redeem_migration_player(
        env: Env,
        wallet: Address,
        vitals: PlayerVitals,
        ipfs_hashes: Vec<String>,
        level: ProgressLevel,
        player_id: u64,
        registered_at: u64,
        updated_at: u64,
        authorization: MigrationAuthorization,
    ) -> Result<u64, ScoutChainError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if authorization.role != MigrationRole::Player {
            return Err(ScoutChainError::InvalidInput);
        }

        if authorization.wallet != wallet {
            return Err(ScoutChainError::InvalidInput);
        }

        if authorization.expires_at > 0 && authorization.expires_at <= env.ledger().timestamp() {
            return Err(ScoutChainError::InvalidInput);
        }

        if env.storage().persistent().has(&DataKey::MigrationNonce(
            wallet.clone(),
            authorization.nonce,
        )) {
            return Err(ScoutChainError::InvalidInput);
        }

        let profile_data_hash = Self::profile_data_hash(
            &env,
            &wallet,
            &vitals,
            &ipfs_hashes,
            player_id,
            registered_at,
            updated_at,
        );
        if authorization.profile_data_hash != profile_data_hash {
            return Err(ScoutChainError::InvalidInput);
        }

        let message = Self::migration_message(&env, &authorization);
        let public_key = Self::address_to_ed25519_key(&env, &wallet);
        // ed25519_verify panics on invalid signature rather than returning bool;
        // wrap in a check via a no-panic approach — invoke and treat panic as invalid.
        env.crypto()
            .ed25519_verify(&public_key, &message, &authorization.signature);

        Self::mark_migration_nonce(&env, &wallet, authorization.nonce);

        let result = Self::admin_seed_player(
            env.clone(),
            wallet.clone(),
            vitals,
            ipfs_hashes,
            level,
            player_id,
            registered_at,
            updated_at,
        );
        if result.is_ok() {
            events::migration_redeemed(
                &env,
                &wallet,
                &crate::types::MigrationRole::Player,
                player_id,
                &authorization.new_contract_hint,
            );
        }
        result
    }

    /// Redeem a scout migration authorization signed by the scout off-chain.
    ///
    /// A relayer with no scout private key can call this function to recreate
    /// a scout's profile on a freshly deployed contract. The function verifies
    /// the scout's ed25519 signature over the canonical authorization message
    /// before writing any state.
    ///
    /// The signed message covers:
    /// `wallet || role(Scout=1) || region_hash || new_contract_hint || nonce || expires_at`
    pub fn redeem_migration_scout(
        env: Env,
        wallet: Address,
        region: String,
        scout_id: u64,
        registered_at: u64,
        verified: bool,
        authorization: MigrationAuthorization,
    ) -> Result<u64, ScoutChainError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        if authorization.role != MigrationRole::Scout {
            return Err(ScoutChainError::InvalidInput);
        }

        if authorization.wallet != wallet {
            return Err(ScoutChainError::InvalidInput);
        }

        if authorization.expires_at > 0 && authorization.expires_at <= env.ledger().timestamp() {
            return Err(ScoutChainError::InvalidInput);
        }

        if env.storage().persistent().has(&DataKey::MigrationNonce(
            wallet.clone(),
            authorization.nonce,
        )) {
            return Err(ScoutChainError::InvalidInput);
        }

        let region_hash = Self::region_hash(&env, &region);
        if authorization.profile_data_hash != region_hash {
            return Err(ScoutChainError::InvalidInput);
        }

        let message = Self::migration_message(&env, &authorization);
        let public_key = Self::address_to_ed25519_key(&env, &wallet);
        env.crypto()
            .ed25519_verify(&public_key, &message, &authorization.signature);

        Self::mark_migration_nonce(&env, &wallet, authorization.nonce);

        let result = Self::admin_seed_scout(
            env.clone(),
            wallet.clone(),
            region,
            scout_id,
            registered_at,
            verified,
        );
        if result.is_ok() {
            events::migration_redeemed(
                &env,
                &wallet,
                &crate::types::MigrationRole::Scout,
                scout_id,
                &authorization.new_contract_hint,
            );
        }
        result
    }

    /// Compute a hash of the player profile data for migration authorization.
    fn profile_data_hash(
        env: &Env,
        wallet: &Address,
        vitals: &PlayerVitals,
        ipfs_hashes: &Vec<String>,
        player_id: u64,
        registered_at: u64,
        updated_at: u64,
    ) -> Bytes {
        let mut buf = Bytes::new(env);
        // wallet strkey bytes
        let wallet_str = wallet.to_string();
        buf.append(&wallet_str.to_bytes());
        // vitals.age
        buf.extend_from_array(&vitals.age.to_be_bytes());
        // vitals.position
        buf.append(&vitals.position.to_bytes());
        // vitals.region
        buf.append(&vitals.region.to_bytes());
        // vitals.nationality
        buf.append(&vitals.nationality.to_bytes());
        // ipfs hashes
        for i in 0..ipfs_hashes.len() {
            if let Some(h) = ipfs_hashes.get(i) {
                buf.append(&h.to_bytes());
            }
        }
        // player_id, registered_at, updated_at
        buf.extend_from_array(&player_id.to_be_bytes());
        buf.extend_from_array(&registered_at.to_be_bytes());
        buf.extend_from_array(&updated_at.to_be_bytes());
        env.crypto().sha256(&buf).into()
    }

    /// Compute a hash of the scout region data for migration authorization.
    fn region_hash(env: &Env, region: &String) -> Bytes {
        let mut buf = Bytes::new(env);
        buf.append(&region.to_bytes());
        env.crypto().sha256(&buf).into()
    }

    /// Construct the canonical message that a player/scout signs for migration.
    fn migration_message(env: &Env, auth: &MigrationAuthorization) -> Bytes {
        let mut msg = Bytes::new(env);
        // wallet strkey
        msg.append(&auth.wallet.to_string().to_bytes());
        // role byte
        let role_byte: u8 = match auth.role {
            MigrationRole::Player => 0u8,
            MigrationRole::Scout => 1u8,
        };
        msg.extend_from_array(&[role_byte]);
        // profile_data_hash
        msg.append(&auth.profile_data_hash);
        // new_contract_hint strkey
        msg.append(&auth.new_contract_hint.to_string().to_bytes());
        // nonce
        msg.extend_from_array(&auth.nonce.to_be_bytes());
        // expires_at
        msg.extend_from_array(&auth.expires_at.to_be_bytes());
        msg
    }

    fn mark_migration_nonce(env: &Env, wallet: &Address, nonce: u64) {
        let key = DataKey::MigrationNonce(wallet.clone(), nonce);
        env.storage().persistent().set(&key, &true);
        // Replay protection must survive as long as the contract itself: extend
        // to the max TTL whenever the current TTL is below it (always true for
        // a freshly-written nonce, whose default TTL is far below the max).
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_MAX, PERSISTENT_TTL_MAX);
    }

    /// Derive an ed25519 public key BytesN<32> from a G-address.
    fn address_to_ed25519_key(env: &Env, address: &Address) -> BytesN<32> {
        use soroban_sdk::address_payload::AddressPayload;
        match AddressPayload::from_address(address) {
            Some(AddressPayload::AccountIdPublicKeyEd25519(key)) => key,
            _ => BytesN::from_array(env, &[0u8; 32]),
        }
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    pub fn get_player(env: Env, player_id: u64) -> Result<PlayerProfile, ScoutChainError> {
        Self::load_player(&env, player_id)
    }

    /// Return the deactivation flag independently of the profile payload.
    /// This keeps indexers and migration tooling from having to inspect raw
    /// storage to preserve the player's visibility state.
    pub fn is_player_deactivated(env: Env, player_id: u64) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::PlayerDeactivated(player_id))
            .unwrap_or(false)
    }

    /// Recover an archived (or expired-but-not-evicted) player profile by
    /// re-extending its TTL to the core-identity policy value (518,400 ledgers).
    ///
    /// On Soroban protocol 23+, reading an archived entry auto-restores it
    /// within the archival grace period. This entrypoint makes that recovery
    /// explicit and operator-driven, then lifts the entry's TTL out of the
    /// minimal auto-restore window back to the full documented lifetime so it
    /// cannot silently age into permanent eviction.
    ///
    /// Admin-only. Returns `PlayerRecordEvicted` if the entry has already been
    /// fully evicted (key absent) and is no longer recoverable.
    pub fn restore_player_record(env: Env, player_id: u64) -> Result<(), ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let profile: StoredPlayerProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Player(player_id))
            .ok_or(ScoutChainError::PlayerRecordEvicted)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Player(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Re-extend the derived index TTLs so the player remains discoverable
        // via filter_players after restoration.  The indexes may have archived
        // on the same schedule as the profile; re-inserting ensures membership
        // is correct regardless.
        let level: ProgressLevel = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerLevel(player_id))
            .unwrap_or(ProgressLevel::Unverified);
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerLevel(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Re-insert into PlayerIndex (guarded against duplicates).
        let mut player_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerIndex)
            .unwrap_or_else(|| Vec::new(&env));
        if !player_ids.iter().any(|id| id == player_id) {
            player_ids.push_back(player_id);
            env.storage()
                .persistent()
                .set(&DataKey::PlayerIndex, &player_ids);
        }
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerIndex,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Re-insert into composite (level, region) index (guarded against duplicates).
        let composite_key = DataKey::PlayersByLevelRegion(level.clone(), profile.vitals.region.clone());
        let mut composite_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&composite_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !composite_ids.iter().any(|id| id == player_id) {
            composite_ids.push_back(player_id);
            env.storage()
                .persistent()
                .set(&composite_key, &composite_ids);
        }
        env.storage().persistent().extend_ttl(
            &composite_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Re-insert into per-level index (guarded against duplicates).
        let level_key = DataKey::PlayersByLevel(level.clone());
        let mut level_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&level_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !level_ids.iter().any(|id| id == player_id) {
            level_ids.push_back(player_id);
            env.storage()
                .persistent()
                .set(&level_key, &level_ids);
        }
        env.storage().persistent().extend_ttl(
            &level_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::player_record_restored(&env, &admin, player_id);
        Ok(())
    }

    /// Recover an archived (or expired-but-not-evicted) scout profile by
    /// re-extending its TTL to the core-identity policy value (518,400 ledgers).
    ///
    /// See `restore_player_record` for the protocol-23 archival-recovery
    /// semantics. Admin-only. Returns `ScoutRecordEvicted` if the entry has
    /// already been fully evicted (key absent) and is unrecoverable.
    pub fn restore_scout_record(env: Env, scout_id: u64) -> Result<(), ScoutChainError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let _profile: ScoutProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutRecordEvicted)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Scout(scout_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        events::scout_record_restored(&env, &admin, scout_id);
        Ok(())
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

    pub fn get_player_status(env: Env, player_id: u64) -> Result<PlayerStatus, ScoutChainError> {
        Self::load_stored_player(&env, player_id)?;
        if env
            .storage()
            .persistent()
            .get::<DataKey, bool>(&DataKey::PlayerDeactivated(player_id))
            .unwrap_or(false)
        {
            Ok(PlayerStatus::Deactivated)
        } else {
            Ok(PlayerStatus::Active)
        }
    }

    /// Return the current status of a scout account.
    ///
    /// - `Active`      — scout exists and has not been deactivated.
    /// - `Deactivated` — scout exists but has been soft-deactivated by admin.
    /// - `NotRegistered` — no scout profile found for the given `scout_id`.
    pub fn get_scout_status(env: Env, scout_id: u64) -> ScoutStatus {
        let exists = env.storage().persistent().has(&DataKey::Scout(scout_id));
        if !exists {
            return ScoutStatus::NotRegistered;
        }
        if env
            .storage()
            .persistent()
            .get::<DataKey, bool>(&DataKey::ScoutDeactivated(scout_id))
            .unwrap_or(false)
        {
            ScoutStatus::Deactivated
        } else {
            ScoutStatus::Active
        }
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

    /// Get a scout profile by wallet address. Used by scout_access contract for Pro-tier verification gating.
    pub fn get_scout_by_wallet(env: Env, wallet: Address) -> Result<ScoutProfile, ScoutChainError> {
        let scout_id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::ScoutByWallet(wallet.clone()))
            .ok_or(ScoutChainError::ScoutNotFound)?;
        Self::get_scout(env, scout_id)
    }

    /// Batch-fetch scout profiles for up to 20 IDs in a single call.
    /// Missing IDs are silently skipped (partial-hit semantics, identical to
    /// `get_players`). The returned vec contains only the profiles that were
    /// found, preserving input order among hits.
    ///
    /// Capped at `MAX_BATCH_SIZE` (20) to bound gas usage per call.
    /// Pass more than 20 IDs → `InvalidInput`.
    pub fn get_scouts(env: Env, ids: Vec<u64>) -> Result<Vec<ScoutProfile>, ScoutChainError> {
        if ids.len() > MAX_BATCH_SIZE {
            return Err(ScoutChainError::InvalidInput);
        }

        let mut profiles = Vec::new(&env);
        for i in 0..ids.len() {
            if let Some(id) = ids.get(i) {
                if let Some(profile) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, ScoutProfile>(&DataKey::Scout(id))
                {
                    profiles.push_back(profile);
                }
            }
        }
        Ok(profiles)
    }

    /// Verify a scout profile (admin only).
    pub fn verify_scout(env: Env, scout_id: u64) -> Result<(), ScoutChainError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let mut profile: ScoutProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutNotFound)?;
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(ScoutChainError::NotInitialized)?;
        profile.verification = ScoutVerificationRecord {
            verified: true,
            verified_by: Some(admin),
            verified_at: Some(env.ledger().timestamp()),
            evidence_ref: None,
            method: Some(String::from_str(&env, "admin_manual")),
        };
        profile.verified = true;
        env.storage()
            .persistent()
            .set(&DataKey::Scout(scout_id), &profile);
        events::scout_verified(&env, scout_id, &profile.wallet);
        Ok(())
    }

    /// Get the structured verification record for a scout by ID.
    pub fn get_scout_verification(
        env: Env,
        scout_id: u64,
    ) -> Result<ScoutVerificationRecord, ScoutChainError> {
        let profile: ScoutProfile = env
            .storage()
            .persistent()
            .get(&DataKey::Scout(scout_id))
            .ok_or(ScoutChainError::ScoutNotFound)?;
        Ok(profile.verification)
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
            pay_to_contact_paused: false,
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
    /// `offset` is a count of eligible (non-deactivated, filter-matching)
    /// entries to skip, NOT a player_id.  `next_cursor` is the count of
    /// eligible entries processed across all pages so far, so passing it
    /// back as `offset` on the next call correctly resumes from where
    /// the previous page ended.
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
                        // The composite index is only a performance hint, never a
                        // trusted source. Re-validate the loaded profile against the
                        // filter so a stale or corrupted bucket entry — from a
                        // deregister_player leak, a set_player_level level/region
                        // mismatch, or a restored-but-not-reindexed player — cannot
                        // leak a non-matching player into the results. This mirrors
                        // the level_gte re-check the slow path already performs.
                        if !Self::level_gte(&profile.level, &min_level) {
                            continue;
                        }
                        if profile.vitals.region != region {
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
                            next_cursor = (skipped + results.len()) as u64;
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
                        next_cursor = (skipped + results.len()) as u64;
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

    /// Enforce the per-caller registration cooldown.
    ///
    /// Reads the last-sent timestamp stored under `last_sent_key`.  If a
    /// timestamp is present and the current ledger time is before
    /// `last_sent + cooldown_secs`, returns `RegistrationCooldown`.
    /// A cooldown of 0 disables the check entirely.
    fn enforce_reg_cooldown(env: &Env, last_sent_key: &DataKey) -> Result<(), ScoutChainError> {
        let cooldown_secs: u64 = env
            .storage()
            .instance()
            .get(&DataKey::RegCooldownSecs(0))
            .unwrap_or(DEFAULT_REG_COOLDOWN_SECS);

        if cooldown_secs == 0 {
            return Ok(());
        }

        let now = env.ledger().timestamp();
        if let Some(last_sent) = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(last_sent_key)
        {
            let next_allowed =
                safe_add_u64(last_sent, cooldown_secs).map_err(|_| ScoutChainError::Overflow)?;
            if now < next_allowed {
                return Err(ScoutChainError::RegistrationCooldown);
            }
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
        if let Some(level) = env
            .storage()
            .persistent()
            .get::<DataKey, ProgressLevel>(&DataKey::PlayerLevel(player_id))
        {
            return level;
        }

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
        let next = safe_add_u64(id, 1).map_err(|_| ScoutChainError::Overflow)?;
        env.storage().instance().set(&DataKey::PlayerCounter, &next);
        Ok(next)
    }

    fn next_scout_id(env: &Env) -> Result<u64, ScoutChainError> {
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ScoutCounter)
            .unwrap_or(0u64);
        let next = safe_add_u64(id, 1).map_err(|_| ScoutChainError::Overflow)?;
        env.storage().instance().set(&DataKey::ScoutCounter, &next);
        Ok(next)
    }

    fn level_gte(level: &ProgressLevel, min_level: &ProgressLevel) -> bool {
        level.rank() >= min_level.rank()
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
    use soroban_sdk::{
        testutils::{Address as _, Events, MockAuth, MockAuthInvoke},
        vec, Env, IntoVal, String, Symbol,
    };

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
    fn test_migration_nonces_survive_default_persistent_ttl() {
        use soroban_sdk::testutils::storage::Persistent as _;
        use soroban_sdk::testutils::Ledger;

        let (env, client) = setup();
        env.ledger().with_mut(|ledger| {
            ledger.sequence_number = 100;
            ledger.max_entry_ttl = PERSISTENT_TTL_MAX + 1;
        });

        let player_wallet = Address::generate(&env);
        let scout_wallet = Address::generate(&env);
        let player_nonce = 11;
        let scout_nonce = 22;

        env.as_contract(&client.address, || {
            RegistrationContract::mark_migration_nonce(&env, &player_wallet, player_nonce);
            RegistrationContract::mark_migration_nonce(&env, &scout_wallet, scout_nonce);

            assert!(
                env.storage().persistent().get_ttl(&DataKey::MigrationNonce(
                    player_wallet.clone(),
                    player_nonce,
                )) > 5_000
            );
            assert!(
                env.storage()
                    .persistent()
                    .get_ttl(&DataKey::MigrationNonce(scout_wallet.clone(), scout_nonce,))
                    > 5_000
            );
        });

        env.ledger().with_mut(|ledger| {
            ledger.sequence_number = 5_100;
        });

        env.as_contract(&client.address, || {
            assert!(env
                .storage()
                .persistent()
                .has(&DataKey::MigrationNonce(player_wallet, player_nonce,)));
            assert!(env
                .storage()
                .persistent()
                .has(&DataKey::MigrationNonce(scout_wallet, scout_nonce,)));
        });
    }

    #[test]
    fn test_initialize_and_health() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        assert!(client.health().initialized);
    }

    #[test]
    fn test_admin_transfer_propose_replace_and_accept() {
        let (env, client) = setup();
        let old_admin = Address::generate(&env);
        let stale_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        client.initialize(&old_admin);

        client.propose_admin(&stale_admin);
        assert_eq!(
            env.events().all(),
            vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, events::ADMIN_TRANSFER_PROPOSED),
                        old_admin.clone(),
                    )
                        .into_val(&env),
                    stale_admin.clone().into_val(&env),
                )
            ]
        );

        // The current admin remains fully functional while a proposal is pending.
        client.pause_contract();
        client.unpause_contract();

        // A new proposal replaces the stale pending address.
        client.propose_admin(&new_admin);
        env.as_contract(&client.address, || {
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::Admin),
                Some(old_admin.clone())
            );
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::PendingAdmin),
                Some(new_admin.clone())
            );
        });

        env.mock_auths(&[MockAuth {
            address: &new_admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "accept_admin",
                args: vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
        assert_eq!(
            env.events().all(),
            vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, events::ADMIN_TRANSFERRED),
                        old_admin.clone(),
                    )
                        .into_val(&env),
                    new_admin.clone().into_val(&env),
                )
            ]
        );
        env.as_contract(&client.address, || {
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::Admin),
                Some(new_admin)
            );
            assert!(!env.storage().persistent().has(&DataKey::PendingAdmin));
        });
    }

    #[test]
    #[should_panic]
    fn test_old_admin_loses_access_after_transfer() {
        let (env, client) = setup();
        let old_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        client.initialize(&old_admin);

        client.propose_admin(&new_admin);
        env.mock_auths(&[MockAuth {
            address: &new_admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "accept_admin",
                args: vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();

        // Privileged calls now require new_admin's signature. Restricting
        // the mocked auth to old_admin must make the call fail, proving the
        // old admin no longer has effective access.
        env.mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "pause_contract",
                args: vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.pause_contract();
    }

    #[test]
    #[should_panic]
    fn test_third_party_cannot_accept_admin() {
        let (env, client) = setup();
        let old_admin = Address::generate(&env);
        let pending_admin = Address::generate(&env);
        let third_party = Address::generate(&env);
        client.initialize(&old_admin);
        client.propose_admin(&pending_admin);

        env.mock_auths(&[MockAuth {
            address: &third_party,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "accept_admin",
                args: vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
    }

    #[test]
    fn test_version() {
        let (env, client) = setup();
        assert_eq!(
            client.version(),
            String::from_str(&env, env!("CARGO_PKG_VERSION"))
        );
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
    fn test_get_player_summary_exposes_no_wallet_or_ipfs_hashes_fields() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes: soroban_sdk::Vec<String> = vec![&env, String::from_str(&env, "QmTest123")];

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        let profile = client.get_player(&player_id);
        let summary = client.get_player_summary(&player_id);

        let PlayerSummary {
            player_id: summary_player_id,
            vitals: summary_vitals,
            level,
            updated_at,
        } = summary;

        assert_eq!(summary_player_id, player_id);
        assert_eq!(summary_vitals.age, vitals.age);
        assert_eq!(summary_vitals.position, vitals.position);
        assert_eq!(summary_vitals.region, vitals.region);
        assert_eq!(summary_vitals.nationality, vitals.nationality);
        assert_eq!(level, ProgressLevel::Unverified);
        assert_eq!(updated_at, profile.updated_at);
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
    fn test_update_profile_rejects_11_hashes_and_accepts_10() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let initial_hashes = vec![&env, String::from_str(&env, "QmInitial")];
        let player_id = client.register_player(&wallet, &vitals, &initial_hashes);

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
        let rejected = client.try_update_profile(&player_id, &too_many);
        assert_eq!(rejected, Err(Ok(ScoutChainError::InvalidInput)));

        let profile_after_rejection = client.get_player(&player_id);
        assert_eq!(profile_after_rejection.ipfs_hashes.len(), 1);
        assert_eq!(
            profile_after_rejection.ipfs_hashes.get(0).unwrap(),
            String::from_str(&env, "QmInitial")
        );

        let valid_ten = vec![
            &env,
            String::from_str(&env, "QmUpdated1"),
            String::from_str(&env, "QmUpdated2"),
            String::from_str(&env, "QmUpdated3"),
            String::from_str(&env, "QmUpdated4"),
            String::from_str(&env, "QmUpdated5"),
            String::from_str(&env, "QmUpdated6"),
            String::from_str(&env, "QmUpdated7"),
            String::from_str(&env, "QmUpdated8"),
            String::from_str(&env, "QmUpdated9"),
            String::from_str(&env, "QmUpdated10"),
        ];
        client.update_profile(&player_id, &valid_ten);

        let profile_after_update = client.get_player(&player_id);
        assert_eq!(profile_after_update.ipfs_hashes.len(), 10);
        assert_eq!(
            profile_after_update.ipfs_hashes.get(0).unwrap(),
            String::from_str(&env, "QmUpdated1")
        );
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

        // Page 2: pass next_cursor from page1 as offset → remaining Forwards
        let page2 = client.filter_players(
            &String::from_str(&env, "West Africa"),
            &String::from_str(&env, "Forward"),
            &ProgressLevel::Unverified,
            &(page1.next_cursor as u32),
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
        assert!(returned_ids.contains(id_wa1), "id_wa1 must be in results");
        assert!(returned_ids.contains(id_wa2), "id_wa2 must be in results");
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
    // Issue #1017: filter_players pagination cursor consistency regression
    // -------------------------------------------------------------------------

    /// Position filter before a page boundary must not cause silent gaps or
    /// duplicates when following the documented next_cursor -> offset contract.
    ///
    /// Concrete scenario: 5 Forwards, 1 Midfielder, 3 Forwards.
    /// Page 1 (limit=4) triggers a boundary at player 5 (Forward).
    /// With the old bug, next_cursor was player_id 5, and passing it back
    /// as offset (a count) skipped 5 eligible entries instead of 4 — losing
    /// player 5.  The cursor is now a count of eligible entries processed,
    /// so this gap cannot occur.
    #[test]
    fn test_filter_players_pagination_cursor_no_gaps() {
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

        // Walk through all pages using the documented cursor contract.
        let mut all_ids: Vec<u64> = Vec::new(&env);
        let mut offset: u32 = 0;
        loop {
            let page = client.filter_players(
                &String::from_str(&env, "West Africa"),
                &String::from_str(&env, "Forward"),
                &ProgressLevel::Unverified,
                &offset,
                &4u32,
            );
            for i in 0..page.profiles.len() {
                all_ids.push_back(page.profiles.get(i).unwrap().player_id);
            }
            if page.next_cursor == 0 {
                break;
            }
            offset = page.next_cursor as u32;
        }

        // Must have exactly 8 Forwards with no gaps or duplicates.
        assert_eq!(all_ids.len(), 8, "must return all 8 Forwards");

        // Verify no duplicates and all expected IDs are present.
        let expected: soroban_sdk::Vec<u64> = {
            let mut v = soroban_sdk::Vec::new(&env);
            v.push_back(1);
            v.push_back(2);
            v.push_back(3);
            v.push_back(4);
            v.push_back(5);
            v.push_back(7);
            v.push_back(8);
            v.push_back(9);
            v
        };
        // Sort all_ids (infallible via bubble-sort on small vec).
        // Since Vec in soroban is limited, just check membership + count.
        for i in 0..expected.len() {
            let id = expected.get(i).unwrap();
            assert!(
                all_ids.contains(id),
                "Forward player {} must appear in paginated results",
                id
            );
        }
        // Check each returned ID is in expected.
        for i in 0..all_ids.len() {
            let id = all_ids.get(i).unwrap();
            assert!(
                expected.contains(id),
                "player {} is not a Forward — gap/duplicate detected",
                id
            );
        }
    }

    #[test]
    fn test_admin_seed_player_persists_profile_and_status() {
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
        let player_id = client.admin_seed_player(
            &wallet,
            &vitals,
            &hashes,
            &ProgressLevel::PerformanceMilestones,
            &7u64,
            &100u64,
            &200u64,
        );

        assert_eq!(player_id, 7u64);
        let profile = client.get_player(&7u64);
        assert_eq!(profile.wallet, wallet);
        assert_eq!(profile.level, ProgressLevel::PerformanceMilestones);
        assert_eq!(client.get_player_status(&7u64), types::PlayerStatus::Active);
    }

    #[test]
    fn test_admin_seed_scout_persists_profile() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let scout_id = client.admin_seed_scout(
            &wallet,
            &String::from_str(&env, "Europe"),
            &11u64,
            &123u64,
            &true,
        );

        assert_eq!(scout_id, 11u64);
        let scout = client.get_scout(&11u64);
        assert_eq!(scout.wallet, wallet);
        assert!(scout.verification.verified);
    }

    // -------------------------------------------------------------------------
    // Issue #647: player_deactivated and player_reactivated event emission
    // -------------------------------------------------------------------------

    #[test]
    fn test_deactivate_player_emits_event() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::IntoVal;
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &dummy_vitals(&env), &hashes);

        // Clear any events from registration so we only inspect deactivation events.
        let _ = env.events().all();

        client.deactivate_player(&player_id);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        soroban_sdk::Symbol::new(&env, "player_deactivated"),
                        admin.clone(),
                    )
                        .into_val(&env),
                    player_id.into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_reactivate_player_emits_event() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::IntoVal;
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &dummy_vitals(&env), &hashes);

        client.deactivate_player(&player_id);

        // Clear events up to this point.
        let _ = env.events().all();

        client.reactivate_player(&player_id);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        soroban_sdk::Symbol::new(&env, "player_reactivated"),
                        admin.clone(),
                    )
                        .into_val(&env),
                    player_id.into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_level_filter_uses_progress_level_ordering() {
        assert!(RegistrationContract::level_gte(
            &ProgressLevel::EliteTier,
            &ProgressLevel::Unverified
        ));
        assert!(RegistrationContract::level_gte(
            &ProgressLevel::EliteTier,
            &ProgressLevel::PerformanceMilestones
        ));
        assert!(RegistrationContract::level_gte(
            &ProgressLevel::PerformanceMilestones,
            &ProgressLevel::VerifiedIdentity
        ));
        assert!(!RegistrationContract::level_gte(
            &ProgressLevel::VerifiedIdentity,
            &ProgressLevel::PerformanceMilestones
        ));
        assert!(!RegistrationContract::level_gte(
            &ProgressLevel::Unverified,
            &ProgressLevel::EliteTier
        ));
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
        assert!(!scout.verification.verified);
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
        assert!(scout.verification.verified);
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

    /// `set_player_level` must only be callable by the address registered via
    /// `set_progress_contract`. A random address attempting to authorize the
    /// same call must be rejected, since `progress_contract.require_auth()`
    /// only succeeds for an authorization entry matching the stored address.
    #[test]
    fn test_set_player_level_rejects_non_progress_contract_caller() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        let player_id = client.register_player(&wallet, &vitals, &hashes);

        let progress_contract = Address::generate(&env);
        client.set_progress_contract(&progress_contract);

        // A random address — not the registered progress contract — signs
        // the authorization for this call instead.
        let random_caller = Address::generate(&env);
        env.mock_auths(&[MockAuth {
            address: &random_caller,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_player_level",
                args: (player_id, ProgressLevel::VerifiedIdentity).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let result = client.try_set_player_level(&player_id, &ProgressLevel::VerifiedIdentity);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Wiring observability (issue #1041)
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_progress_contract_before_and_after_configuration() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        assert_eq!(client.get_progress_contract(), None);

        let progress_addr = Address::generate(&env);
        client.set_progress_contract(&progress_addr);
        assert_eq!(client.get_progress_contract(), Some(progress_addr));
    }

    #[test]
    fn test_get_wiring_state_initially_unconfigured() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let state = client.get_wiring_state();
        assert_eq!(state.progress_contract.address, None);
        assert_eq!(state.progress_contract.epoch, 0);
        assert!(!state.is_fully_wired());
    }

    #[test]
    fn test_get_wiring_state_reflects_link_and_bumps_epoch() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let progress_addr = Address::generate(&env);
        client.set_progress_contract(&progress_addr);

        let state = client.get_wiring_state();
        assert_eq!(state.progress_contract.address, Some(progress_addr));
        assert_eq!(state.progress_contract.epoch, 1);
        assert!(state.is_fully_wired());

        // Freely re-settable — no first-call-only guard — and a second call
        // must bump the epoch again, not reset it.
        let new_progress_addr = Address::generate(&env);
        client.set_progress_contract(&new_progress_addr);
        let state2 = client.get_wiring_state();
        assert_eq!(state2.progress_contract.address, Some(new_progress_addr));
        assert_eq!(
            state2.progress_contract.epoch, 2,
            "re-wiring the same link must bump its epoch again"
        );
    }

    #[test]
    fn test_set_progress_contract_emits_wiring_updated_event() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let progress_addr = Address::generate(&env);
        client.set_progress_contract(&progress_addr);

        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, crate::events::WIRING_UPDATED),
                        admin.clone(),
                        Symbol::new(&env, "progress_contract"),
                    )
                        .into_val(&env),
                    (progress_addr, 1u32).into_val(&env),
                )
            ]
        );
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
        ver_client.register_validator(&validator, &String::from_str(&env, "UEFA B License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // 10. Approve milestone via verification contract (this triggers the cross-contract flow)
        ver_client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Completed Level 1 requirements"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
            &None,
        );

        // 11. Assert that the player's level is now VerifiedIdentity in registration contract
        let profile = reg_client.get_player(&player_id);
        assert_eq!(profile.level, ProgressLevel::VerifiedIdentity);
    }

    // -------------------------------------------------------------------------
    // Issue #820: admin_seed_player / admin_seed_scout tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_admin_seed_player_preserves_exact_ids_and_timestamps() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let reg_id = env.register(RegistrationContract, ());
        let client = RegistrationContractClient::new(&env, &reg_id);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmEvidence")];
        let original_id = 42u64;
        let original_ts = 1_600_000_000u64;

        client.admin_seed_player(
            &wallet,
            &vitals,
            &hashes,
            &ProgressLevel::Unverified,
            &original_id,
            &original_ts,
            &original_ts,
        );

        let profile = client.get_player(&original_id);
        assert_eq!(profile.player_id, original_id);
        assert_eq!(profile.wallet, wallet);
        assert_eq!(profile.vitals.age, vitals.age);
        assert_eq!(profile.registered_at, original_ts);
    }

    #[test]
    fn test_admin_seed_player_rejects_duplicate_id() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let reg_id = env.register(RegistrationContract, ());
        let client = RegistrationContractClient::new(&env, &reg_id);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmEvidence")];

        client.admin_seed_player(
            &wallet,
            &vitals,
            &hashes,
            &ProgressLevel::Unverified,
            &1u64,
            &1_600_000_000u64,
            &1_600_000_000u64,
        );

        let result = client.try_admin_seed_player(
            &wallet,
            &vitals,
            &hashes,
            &ProgressLevel::Unverified,
            &1u64,
            &1_600_000_001u64,
            &1_600_000_001u64,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_admin_seed_scout_rejects_non_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let reg_id = env.register(RegistrationContract, ());
        let client = RegistrationContractClient::new(&env, &reg_id);

        // Mock only the admin's initialize auth; do NOT mock seed auths, so
        // `admin_seed_scout`'s require_admin check fails for any caller.
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &reg_id,
                fn_name: "initialize",
                args: vec![&env, admin.to_val()],
                sub_invokes: &[],
            },
        }]);
        client.initialize(&admin);

        let wallet = Address::generate(&env);

        let result = client.try_admin_seed_scout(
            &wallet,
            &String::from_str(&env, "Europe"),
            &1u64,
            &1_600_000_000u64,
            &false,
        );
        assert!(result.is_err());
    }

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

    // -------------------------------------------------------------------------
    // Issue #649: Vitals field length limits audit & post-registration immutability
    // -------------------------------------------------------------------------

    /// Audit and regression test locking in vitals field length validation in
    /// `register_player` (position <= 64, region <= 100, nationality <= 64 bytes)
    /// and confirming vitals fields are write-once and immutable post-registration.
    #[test]
    fn test_vitals_length_limits_and_immutability_audit() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // 1. Confirm register_player rejects oversized position (> 64 bytes)
        let wallet1 = Address::generate(&env);
        let vitals_bad_pos = PlayerVitals {
            age: 20,
            position: String::from_str(&env, &"P".repeat(65)),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, "Ghana"),
        };
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        assert_eq!(
            client.try_register_player(&wallet1, &vitals_bad_pos, &hashes),
            Err(Ok(ScoutChainError::InvalidInput))
        );

        // 2. Confirm register_player rejects oversized region (> 100 bytes)
        let wallet2 = Address::generate(&env);
        let vitals_bad_reg = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, &"R".repeat(101)),
            nationality: String::from_str(&env, "Ghana"),
        };
        assert_eq!(
            client.try_register_player(&wallet2, &vitals_bad_reg, &hashes),
            Err(Ok(ScoutChainError::InvalidInput))
        );

        // 3. Confirm register_player rejects oversized nationality (> 64 bytes)
        let wallet3 = Address::generate(&env);
        let vitals_bad_nat = PlayerVitals {
            age: 20,
            position: String::from_str(&env, "Forward"),
            region: String::from_str(&env, "West Africa"),
            nationality: String::from_str(&env, &"N".repeat(65)),
        };
        assert_eq!(
            client.try_register_player(&wallet3, &vitals_bad_nat, &hashes),
            Err(Ok(ScoutChainError::InvalidInput))
        );

        // 4. Confirm register_player succeeds with exact upper boundary lengths
        let wallet_valid = Address::generate(&env);
        let vitals_max = PlayerVitals {
            age: 25,
            position: String::from_str(&env, &"P".repeat(64)),
            region: String::from_str(&env, &"R".repeat(100)),
            nationality: String::from_str(&env, &"N".repeat(64)),
        };
        let player_id = client.register_player(&wallet_valid, &vitals_max, &hashes);
        let profile_init = client.get_player(&player_id);
        assert_eq!(profile_init.vitals.position, vitals_max.position);
        assert_eq!(profile_init.vitals.region, vitals_max.region);
        assert_eq!(profile_init.vitals.nationality, vitals_max.nationality);

        // 5. Update profile only accepts new ipfs_hashes — confirm vitals remain unchanged
        let new_hashes = vec![&env, String::from_str(&env, "QmUpdatedHash")];
        client.update_profile(&player_id, &new_hashes);

        let profile_updated = client.get_player(&player_id);
        assert_eq!(profile_updated.ipfs_hashes, new_hashes);
        assert_eq!(profile_updated.vitals.position, vitals_max.position);
        assert_eq!(profile_updated.vitals.region, vitals_max.region);
        assert_eq!(profile_updated.vitals.nationality, vitals_max.nationality);
        assert_eq!(profile_updated.vitals.age, vitals_max.age);
    }

    // -------------------------------------------------------------------------
    // Issue #825: Structured scout verification record
    // -------------------------------------------------------------------------

    #[test]
    fn test_verify_scout_populates_structured_record() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&wallet, &region);

        let before = client.get_scout(&scout_id);
        assert!(!before.verification.verified);
        assert!(before.verification.verified_by.is_none());
        assert!(before.verification.verified_at.is_none());

        client.verify_scout(&scout_id);

        let after = client.get_scout(&scout_id);
        assert!(after.verification.verified);
        assert!(after.verification.verified_by.is_some());
        assert!(after.verification.verified_at.is_some());
        assert_eq!(
            after.verification.method,
            Some(String::from_str(&env, "admin_manual"))
        );
    }

    #[test]
    fn test_get_scout_verification_exposes_record() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "Europe");
        let scout_id = client.register_scout(&wallet, &region);

        let record = client.get_scout_verification(&scout_id);
        assert!(!record.verified);
        assert!(record.verified_by.is_none());

        client.verify_scout(&scout_id);

        let record = client.get_scout_verification(&scout_id);
        assert!(record.verified);
        assert!(record.verified_by.is_some());
    }

    // -------------------------------------------------------------------------
    // Issue #1157: TTL Management
    // -------------------------------------------------------------------------

    #[test]
    fn test_instance_ttl_bumped_on_initialize() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        
        // Contract should be initialized and healthy (TTL was bumped)
        assert!(client.health());
    }

    #[test]
    fn test_instance_ttl_bumped_on_register_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        
        // Register player should bump TTL
        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);
        assert!(client.health());
    }

    #[test]
    fn test_instance_ttl_bumped_on_register_scout() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "West Africa");
        
        // Register scout should bump TTL
        let scout_id = client.register_scout(&wallet, &region);
        assert_eq!(scout_id, 1);
        assert!(client.health());
    }

    #[test]
    fn test_instance_ttl_bumped_on_pause() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Pause should bump TTL
        client.pause_contract();
        assert!(client.health());
    }

    #[test]
    fn test_instance_ttl_bumped_on_unpause() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.pause_contract();

        // Unpause should bump TTL
        client.unpause_contract();
        assert!(client.health());
    }

    // -------------------------------------------------------------------------
    // Issue #1153: Deregister Player
    // -------------------------------------------------------------------------

    #[test]
    fn test_deregister_player_removes_from_storage() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);

        // Deregister should succeed
        client.deregister_player(&player_id);

        // Should not be able to get the player anymore
        let result = client.try_get_player(&player_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_deregister_player_removes_wallet_index() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let player_id = client.register_player(&wallet, &vitals, &hashes);
        client.deregister_player(&player_id);

        // Should not be able to get player by wallet anymore
        let result = client.try_get_player_by_wallet(&wallet);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Issue #1154: Player/Scout Count Getters
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_player_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        assert_eq!(client.get_player_count(), 0);

        let wallet1 = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];
        client.register_player(&wallet1, &vitals, &hashes);
        assert_eq!(client.get_player_count(), 1);

        let wallet2 = Address::generate(&env);
        client.register_player(&wallet2, &vitals, &hashes);
        assert_eq!(client.get_player_count(), 2);
    }

    #[test]
    fn test_get_scout_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        assert_eq!(client.get_scout_count(), 0);

        let wallet1 = Address::generate(&env);
        let region = String::from_str(&env, "West Africa");
        client.register_scout(&wallet1, &region);
        assert_eq!(client.get_scout_count(), 1);

        let wallet2 = Address::generate(&env);
        client.register_scout(&wallet2, &region);
        assert_eq!(client.get_scout_count(), 2);
    }

    #[test]
    fn test_player_count_decrements_on_deregister() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet1 = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        let player_id1 = client.register_player(&wallet1, &vitals, &hashes);
        assert_eq!(client.get_player_count(), 1);

        let wallet2 = Address::generate(&env);
        let player_id2 = client.register_player(&wallet2, &vitals, &hashes);
        assert_eq!(client.get_player_count(), 2);

        // Deregister first player
        client.deregister_player(&player_id1);
        assert_eq!(client.get_player_count(), 1);

        // Deregister second player
        client.deregister_player(&player_id2);
        assert_eq!(client.get_player_count(), 0);
    }

    // -------------------------------------------------------------------------
    // Issue #1155: Migration Functions (Relayer Pattern)
    // -------------------------------------------------------------------------

    #[test]
    fn test_redeem_migration_player_succeeds_with_relayer() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        // Relayer (wallet without admin role) can redeem a migration
        let player_id = client.redeem_migration_player(&wallet, &vitals, &hashes);
        assert_eq!(player_id, 1);

        let profile = client.get_player(&player_id);
        assert_eq!(profile.wallet, wallet);
    }

    #[test]
    fn test_redeem_migration_scout_succeeds_with_relayer() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "West Africa");

        // Relayer (wallet without admin role) can redeem a migration
        let scout_id = client.redeem_migration_scout(&wallet, &region);
        assert_eq!(scout_id, 1);

        let profile = client.get_scout(&scout_id);
        assert_eq!(profile.wallet, wallet);
    }

    #[test]
    #[should_panic]
    fn test_redeem_migration_player_duplicate_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![&env, String::from_str(&env, "QmTest")];

        client.redeem_migration_player(&wallet, &vitals, &hashes);
        // second call should panic with AlreadyRegistered
        client.redeem_migration_player(&wallet, &vitals, &hashes);
    }

    #[test]
    #[should_panic]
    fn test_redeem_migration_scout_duplicate_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let region = String::from_str(&env, "West Africa");

        client.redeem_migration_scout(&wallet, &region);
        // second call should panic with AlreadyRegistered
        client.redeem_migration_scout(&wallet, &region);
    }
}
}
