#![cfg_attr(target_family = "wasm", no_std)]
#![no_std]
#![no_std]

mod errors;
mod events;
mod types;

use errors::ProgressError;
use events::*;
use types::*;

use soroban_sdk::{contract, contractimpl, Address, Env, Vec};

const INSTANCE_TTL_MIN: u32 = 500;
const INSTANCE_TTL_MAX: u32 = 500;
const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 2000;

const ADMIN_BUMP_LEDGERS: u32 = 518400; // ~30 days at 5s/ledger
pub use errors::ProgressError;
use scoutchain_shared_types::{
    require_admin, safe_math::safe_add_u32, write_wiring_link, ContractHealth, ProgressLevel,
};
pub use types::{DataKey, HistoryProofStep, ProgressEntry, ProgressWiringState};

use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, String, Vec};

const INSTANCE_TTL_MIN: u32 = 100;
const INSTANCE_TTL_MAX: u32 = 500;

// Core identity TTL: 30 days at ~5s/ledger ≈ 518_400 ledgers.
// PlayerLevel is core identity data that must survive extended dormancy.
// Players building reputation over months should not silently lose their level.
const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 518_400;

// Admin key bumped conservatively; syncs with registration contract to ensure
// cross-contract admin operations remain valid.
const ADMIN_BUMP_LEDGERS: u32 = 518_400;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const HISTORY_PAGE_SIZE: u32 = 8;

// Minimal client for the registration contract.
// Used to sync a player's level after advance_level / reset_player_level.
mod registration_contract {
    use scoutchain_shared_types::ProgressLevel;
    use soroban_sdk::{contractclient, contracterror, Env};

    #[contracterror]
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(u32)]
    pub enum Error {
        PlayerNotFound = 3,
        Unauthorized = 10,
    }

    #[contractclient(name = "Client")]
    #[allow(dead_code)]
    pub trait RegistrationContractClient {
        fn set_player_level(env: Env, player_id: u64, level: ProgressLevel) -> Result<(), Error>;
    }
}

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
    pub fn initialize(env: Env, admin: Address) -> Result<(), ProgressError> {
        if Self::is_initialized(&env) {
            return Err(ProgressError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, ADMIN_BUMP_LEDGERS, ADMIN_BUMP_LEDGERS);
        Self::bump_instance_ttl(&env);
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

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
        new_level: ProgressLevel,
    ) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        Self::bump_instance_ttl(&env);

        // Fetch and check the player's current level
        let player_key = DataKey::Player(player_id);
        let player: Player = env
            .storage()
            .instance()
            .get(&player_key)
            .ok_or(ProgressError::NotFound)?;

        if player.level == new_level {
            return Err(ProgressError::InvalidLevel);
        }

        let old_level = player.level;

        // Update the player's level
        let mut updated_player = player;
        updated_player.level = new_level;
        env.storage().instance().set(&player_key, &updated_player);

        // Record history entry for the reset
        let reset_record = LevelChangeHistory {
            old_level,
            new_level,
            reason: ResetReason::DisputeResolution,
            timestamp: env.ledger().timestamp(),
        };

        let history_key = DataKey::History(player_id, player.history_count + 1);
        env.storage().instance().set(&history_key, &reset_record);

        // Increment history counter
        let new_history_count = player.history_count + 1;
        env.storage().instance().set(
            &DataKey::HistoryCount(player_id),
            &new_history_count,
        );

        // Update player to new history count
        updated_player.history_count = new_history_count;
        env.storage().instance().set(&player_key, &updated_player);

        events::player_level_reset(&env, player_id, old_level, new_level);
        Ok(())
    }

    /// Store the registration contract address so we can sync player levels (admin only).
    pub fn set_registration_contract(env: Env, addr: Address) -> Result<(), ProgressError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::RegistrationContract,
            &DataKey::RegistrationContractEpoch,
            &addr,
        );
        events::wiring_updated(&env, &admin, "registration_contract", &addr, epoch);
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    /// Store the verification contract address so advance_level can validate
    /// that the caller is the configured VerificationContract (admin only).
    pub fn set_verification_contract(env: Env, addr: Address) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::VerificationContract,
            &DataKey::VerificationContractEpoch,
            &addr,
        );
        events::wiring_updated(&env, &admin, "verification_contract", &addr, epoch);
        Ok(())
    }

    /// Optionally whitelist the scout_access contract as a secondary caller of
    /// advance_level (for trial-offer Level-3 advances). Admin only.
    pub fn set_scout_access_contract(env: Env, addr: Address) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::ScoutAccessContract,
            &DataKey::ScoutAccessContractEpoch,
            &addr,
        );
        events::wiring_updated(&env, &admin, "scout_access_contract", &addr, epoch);
        Ok(())
    }

    /// Return the configured verification contract address, or `None` if the
    /// link has not been configured. Read-only and requires no auth.
    pub fn get_verification_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::VerificationContract)
    }

    /// Return the configured registration contract address, or `None` if the
    /// link has not been configured. Read-only and requires no auth.
    pub fn get_registration_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::RegistrationContract)
    }

    /// Return the configured scout_access contract address, or `None` if the
    /// link has not been configured. Read-only and requires no auth.
    pub fn get_scout_access_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::ScoutAccessContract)
    }

    /// Propose a replacement administrator. The current admin remains active
    /// until the proposed address calls `accept_admin`.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
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
    pub fn accept_admin(env: Env) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ProgressError::NotInitialized)?;
        let new_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .ok_or(ProgressError::PendingAdminNotSet)?;
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
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), ProgressError> {
        Self::propose_admin(env, new_admin)
    }

    /// Upgrade the contract WASM. Admin auth required.
    /// Persistent storage (including Admin) survives this call.
    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) -> Result<(), ProgressError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
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
        Self::require_initialized(&env)?;
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let old_level = Self::get_current_level(&env, player_id);
        
        // Validate that target_level < current_level (true rollback, not same-level or forward jump)
        let old_code = Self::level_code(&old_level);
        let target_code = Self::level_code(&target_level);
        if target_code >= old_code {
            return Err(ProgressError::InvalidProgressTransition);
        }
        
        Self::record_progress_entry(
            &env,
            player_id,
            old_level.clone(),
            target_level.clone(),
            admin.clone(),
            0,
        )?;
        env.storage()
            .persistent()
            .set(&DataKey::PlayerLevel(player_id), &target_level);
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerLevel(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

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

        events::player_level_reset(&env, &admin, player_id, &old_level, &target_level);
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
        validator: Address,
        player_id: u64,
        validator_id: u32,
    ) -> Result<(), ProgressError> {
        Self::bump_instance_ttl(&env);
        validator.require_auth();

        // Confirm the validator has registered and is active
        Self::check_validator(&env, &validator)?;

        // Fetch the player or create a new one
        let player_key = DataKey::Player(player_id);
        let mut player: Player = env
            .storage()
            .instance()
            .get(&player_key)
            .unwrap_or_else(|| Player {
                id: player_id,
                level: ProgressLevel::Unverified,
                history_count: 0,
            });

        // Advance to the next level
        player.level = player.level.next();
        env.storage().instance().set(&player_key, &player);

        // Record the history
        let old_level = player.level;
        let new_level = player.level;
        let history_key = DataKey::History(player_id, player.history_count + 1);

        let history = LevelChangeHistory {
            old_level,
            new_level,
            reason: ResetReason::ValidatorApproval,
            timestamp: env.ledger().timestamp(),
        };

        env.storage().instance().set(&history_key, &history);

        // Increment history counter
        let new_history_count = player.history_count + 1;
        env.storage().instance().set(
            &DataKey::HistoryCount(player_id),
            &new_history_count,
        );

        events::player_level_advanced(&env, player_id, old_level, new_level, validator, validator_id);
        Ok(())
    }

    pub fn register_validator(
        env: Env,
        validator: Address,
        name: String,
    ) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        Self::bump_instance_ttl(&env);
        validator.require_auth();

        if name.len() > 256 {
            return Err(ProgressError::InvalidLength);
        }

        if env
            .storage()
            .instance()
            .has(&DataKey::Validator(validator.clone()))
        {
            return Err(ProgressError::AlreadyExists);
        }

        env.storage()
            .instance()
            .set(&DataKey::Validator(validator.clone()), &name);
        env.storage()
            .instance()
            .set(&DataKey::ValidatorActive(validator.clone()), &true);

        // Add to validator vector
        let mut validators: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));
        validators.push_back(validator);
        milestone_ref: u32,
    ) -> Result<ProgressLevel, ProgressError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        // Only the configured VerificationContract (or the optional secondary
        // ScoutAccessContract for trial-offer Level-3 advances) may call this
        // function.  If neither whitelist address is configured the call is
        // rejected — there is no open fallback.
        let verification_contract: Option<Address> = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::VerificationContract);

        // Check whether an optional secondary caller (e.g. scout_access) is
        // also whitelisted, then require auth from whichever address matches.
        let secondary: Option<Address> = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ScoutAccessContract);

        let caller_is_secondary = secondary.as_ref().map(|a| a == &caller).unwrap_or(false);

        if caller_is_secondary {
            secondary.unwrap().require_auth();
        } else if let Some(ref vc) = verification_contract {
            vc.require_auth();
        } else {
            return Err(ProgressError::NotInitialized);
        }

        // #457: When called via the secondary (ScoutAccessContract) path,
        // validate that milestone_ref actually exists on-chain for this
        // player. A milestone_ref of 0 or one beyond the known count is
        // rejected with InvalidProgressTransition, preventing fabricated
        // indices from advancing a player's level.
        //
        // This check is skipped for the primary VerificationContract caller:
        // that contract is the source of truth for milestone data (it calls
        // advance_level directly from approve_milestone with a milestone_ref
        // it just created), so re-validating would both be redundant and,
        // because VerificationContract would still be on the call stack,
        // trigger a disallowed contract re-entry when advance_level called
        // back into it.
        if caller_is_secondary {
            let ver_addr = verification_contract
                .as_ref()
                .ok_or(ProgressError::NotInitialized)?;
            let ver_client = verification_contract::Client::new(&env, ver_addr);
            let count = ver_client.get_milestone_count(&player_id);
            if milestone_ref == 0 || milestone_ref > count {
                return Err(ProgressError::InvalidProgressTransition);
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
            .instance()
            .set(&DataKey::ValidatorVector, &validators);

        events::validator_registered(&env, validator);
        Ok(())
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

    pub fn revoke_validator(env: Env, validator: Address) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        Self::bump_instance_ttl(&env);

        if !env
            .storage()
            .instance()
            .has(&DataKey::Validator(validator.clone()))
        {
            return Err(ProgressError::NotFound);
        }

        env.storage()
            .instance()
            .set(&DataKey::ValidatorActive(validator.clone()), &false);

        events::validator_revoked(&env, validator);
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        Self::bump_instance_ttl(&env);
        env.storage().instance().set(&DataKey::Paused, &true);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), ProgressError> {
        Self::require_admin(&env)?;
        Self::bump_instance_ttl(&env);
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn get_level(env: Env, player_id: u64) -> ProgressLevel {
        let player: Player = env
            .storage()
            .instance()
            .get(&DataKey::Player(player_id))
            .unwrap_or_else(|| Player {
                id: player_id,
                level: ProgressLevel::Unverified,
                history_count: 0,
            });

        player.level
    }

    pub fn get_player(env: Env, player_id: u64) -> Player {
        env.storage()
            .instance()
            .get(&DataKey::Player(player_id))
            .unwrap_or_else(|| Player {
                id: player_id,
                level: ProgressLevel::Unverified,
                history_count: 0,
            })
    }

    pub fn get_milestone_count(env: Env, player_id: u64) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::HistoryCount(player_id))
            .unwrap_or(0)
    }

    pub fn get_milestone(env: Env, player_id: u64, index: u32) -> LevelChangeHistory {
        env.storage()
            .instance()
            .get(&DataKey::History(player_id, index))
            .unwrap_or_else(|| LevelChangeHistory {
                old_level: ProgressLevel::Unverified,
                new_level: ProgressLevel::Unverified,
                reason: ResetReason::DisputeResolution,
                timestamp: 0,
            })
    }

    pub fn is_active_validator(env: Env, validator: Address) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::ValidatorActive(validator))
            .unwrap_or(false)
    }

    pub fn get_validator_name(env: Env, validator: Address) -> String {
        env.storage()
            .instance()
            .get::<_, String>(&DataKey::Validator(validator))
            .unwrap_or_else(|| String::from_str(&env, ""))
    }

    pub fn get_validators(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn health(env: Env) -> ContractHealth {
        let is_initialized = Self::is_initialized(&env);
        let is_paused = Self::is_paused(&env);

        ContractHealth {
            name: String::from_str(&env, "ProgressContract"),
            initialized: is_initialized,
            paused: is_paused,
        }
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn is_initialized(env: &Env) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
    }

    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
    }

    fn check_validator(env: &Env, validator: &Address) -> Result<(), ProgressError> {
        if !env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::ValidatorActive(validator.clone()))
            .unwrap_or(false)
        {
            return Err(ProgressError::NotAuthorized);
        let key = &DataKey::PlayerLevel(player_id);
        let level = env
            .storage()
            .persistent()
            .get(key)
            .unwrap_or(ProgressLevel::Unverified);

        // Keep-alive: extend TTL on any read to prevent silent archival of dormant players.
        // This is cheaper than losing a player's reputation to archival decay.
        //
        // The `has` guard is required: `extend_ttl` on a key that was never
        // written raises Storage/MissingValue, which the host escalates to a
        // panic. Without it, reading a player that has never advanced (the
        // documented `Unverified` default) would trap instead of returning.
        if env.storage().persistent().has(key) {
            env.storage()
                .persistent()
                .extend_ttl(key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        level
    }

    /// Recover an archived (or expired-but-not-evicted) player-level entry by
    /// re-extending its TTL to the core-identity policy value (518,400 ledgers).
    ///
    /// On Soroban protocol 23+, reading an archived entry auto-restores it
    /// within the archival grace period. This entrypoint makes that recovery
    /// explicit and operator-driven, then lifts the entry's TTL back to the
    /// full documented lifetime so it cannot silently age into permanent
    /// eviction.
    ///
    /// Admin-only. Returns `PlayerLevelRecordEvicted` if the entry has already
    /// been fully evicted (key absent) and is unrecoverable.
    pub fn restore_player_level_record(env: Env, player_id: u64) -> Result<(), ProgressError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let _level: ProgressLevel = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerLevel(player_id))
            .ok_or(ProgressError::PlayerLevelRecordEvicted)?;
        env.storage().persistent().extend_ttl(
            &DataKey::PlayerLevel(player_id),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        events::player_level_record_restored(&env, &admin, player_id);
        Ok(())
    }

    pub fn get_history_count(env: Env, player_id: u64) -> u32 {
        Self::bump_instance_ttl(&env);
        let key = DataKey::HistoryCounter(player_id);
        let count: u32 = env.storage().persistent().get(&key).unwrap_or(0u32);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        count
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
    /// The on-chain layout is now a bounded set of fixed-size `HistoryPage`
    /// shards rather than one unbounded `HistoryVec` key. The function
    /// reconstructs the logical history from those pages and keeps the
    /// per-read/storage cost bounded by the page size instead of the full
    /// historical count.
    /// Returns an empty Vec if the player has no history.
    pub fn get_progress_history(env: Env, player_id: u64) -> Vec<ProgressEntry> {
        let history = Self::read_history_pages(&env, player_id);
        let page_count =
            (history.len() as u32).saturating_add(HISTORY_PAGE_SIZE - 1) / HISTORY_PAGE_SIZE;
        for page_index in 0..page_count {
            let key = DataKey::HistoryPage(player_id, page_index);
            if env.storage().persistent().has(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
            }
        }
        history
    }

    /// Paginated history retrieval. Returns entries from `offset+1` to `offset+limit`.
    /// `limit` is clamped to 1..=50. Returns an empty Vec when `offset` >= total count.
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

        // Read-side keep-alive (issue #1191): a player whose history is only
        // ever read through the paginated getters must not have that history
        // archived. Extend the counter so the read path itself keeps it alive,
        // matching the write-side and `get_progress_history` behaviour.
        if count > 0 {
            env.storage().persistent().extend_ttl(
                &DataKey::HistoryCounter(player_id),
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        if offset >= count {
            return Vec::new(&env);
        }

        let effective_limit = limit.clamp(1, MAX_PAGE);
        let start = offset + 1; // entries are 1-indexed
        let end = (start + effective_limit - 1).min(count);

        let mut entries: Vec<ProgressEntry> = Vec::new(&env);
        for i in start..=end {
            let key = DataKey::HistoryEntry(player_id, i);
            if let Some(entry) = env.storage().persistent().get(&key) {
                env.storage().persistent().extend_ttl(
                    &key,
                    PERSISTENT_TTL_MIN,
                    PERSISTENT_TTL_MAX,
                );
                entries.push_back(entry);
            }
        }
        entries
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

use soroban_sdk::String;
use scoutchain_shared_types::{ContractHealth, ProgressLevel};

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{Address, Env, String};

    fn setup() -> (Env, Address, scoutchain_progress_contract::Client<'static>) {
        let env = Env::default();
        let contract_id = env.register_contract(None, ProgressContract);
        let client = scoutchain_progress_contract::Client::new(&env, &contract_id);

        (env, contract_id, client)
    }

    #[test]
    fn test_initialize() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);

        assert!(!env
            .storage()
            .instance()
            .has(&DataKey::Initialized));

        client.initialize(&admin);

        assert!(env
            .storage()
            .instance()
            .has(&DataKey::Initialized));
    }

    #[test]
    fn test_initialize_twice_fails() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);

        client.initialize(&admin);

        let result = client.try_initialize(&admin);
        assert!(result.is_err());
    }

    #[test]
    fn test_pause_unpause() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        assert!(!client.is_paused());
        client.pause_contract();
        assert!(client.is_paused());
        client.unpause_contract();
        assert!(!client.is_paused());
    }

    #[test]
    fn test_register_validator() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let name = String::from_str(&env, "Alice");

        client.register_validator(&validator, &name);

        assert!(client.is_active_validator(&validator));
    }

    #[test]
    fn test_advance_level_success() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let player_id = 1u64;
        let validator = Address::generate(&env);
        let validator_name = String::from_str(&env, "Coach");

        client.register_validator(&validator, &validator_name);

        assert_eq!(client.get_level(player_id), ProgressLevel::Unverified);

        client.advance_level(&validator, &player_id, &1u32);

        assert_eq!(client.get_level(player_id), ProgressLevel::VerifiedIdentity);
    }

    #[test]
    fn test_upgrade_preserves_admin() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach"));
        client.advance_level(&validator, &1u64, &1u32);

        let new_wasm_hash = env.deployer().upload_contract_wasm(soroban_sdk::Bytes::new(&env));
        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.pause_contract();
        // Player level data persisted
        assert_eq!(client.get_level(&1u64), ProgressLevel::VerifiedIdentity);
    }

    #[test]
    fn test_reset_player_level_success() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let player_id = 1u64;
        let validator = Address::generate(&env);

        client.register_validator(&validator, &String::from_str(&env, "Coach"));
        client.advance_level(&validator, &player_id, &1u32);

        let player = client.get_player(&player_id);
        assert_eq!(player.level, ProgressLevel::VerifiedIdentity);

        client.reset_player_level(&player_id, &ProgressLevel::Unverified);

        let reset_player = client.get_player(&player_id);
        assert_eq!(reset_player.level, ProgressLevel::Unverified);
    }

    #[test]
    #[should_panic]
    fn test_subscription_expiry() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let validator_name = String::from_str(&env, "Coach");
        client.register_validator(&validator, &validator_name);

        let player_id = 1u64;
        client.advance_level(&validator, &player_id, &1u32);

        let player = client.get_player(&player_id);
        assert_eq!(player.history_count, 1);
    }

    #[test]
    fn test_reset_player_level_to_same_level_fails() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let player_id = 1u64;

        let result = client.try_reset_player_level(&player_id, &ProgressLevel::Unverified);
        assert!(result.is_err());
    }

    #[test]
    fn test_transfer_admin() {
        let (env, _, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
    /// Stable cursor-based history pagination.
    ///
    /// Guarantees that a consumer paging through results sees **every entry
    /// exactly once** even if new entries are appended between page fetches —
    /// solving the skip/duplicate problem that plain `offset` pagination has
    /// under concurrent mutation (issue #800).
    ///
    /// ## How it works
    ///
    /// On the **first call** pass `cursor_snapshot` = `None` and
    /// `cursor_next_index` = `None`. The function snapshots the current
    /// entry count and returns entries `1..=min(limit, snapshot_count)`.
    /// It also returns the `snapshot_count` and the `next_index` for the
    /// following page.
    ///
    /// On **subsequent calls** pass back the `snapshot_count` and
    /// `next_index` values from the previous response. The function uses
    /// `snapshot_count` as the immutable upper bound — any entries appended
    /// after the first call are invisible to this cursor, so no entry is
    /// skipped or duplicated.
    ///
    /// The cursor is fully self-describing (two u32 values) and requires no
    /// server-side session state.
    ///
    /// ## Parameters
    ///
    /// - `player_id`         — the player whose history to page through.
    /// - `cursor_snapshot`   — `None` for the first page; `Some(snapshot_count)`
    ///                         from the previous response for subsequent pages.
    /// - `cursor_next_index` — `None` for the first page; `Some(next_index)`
    ///                         from the previous response for subsequent pages.
    /// - `limit`             — max entries per page (capped at 50).
    ///
    /// ## Returns `(entries, next_index, snapshot_count)`
    ///
    /// - `entries`        — the page of [`ProgressEntry`] records.
    /// - `next_index`     — pass as `cursor_next_index` in the next call.
    ///                      `0` signals that all entries have been consumed.
    /// - `snapshot_count` — pass as `cursor_snapshot` in the next call
    ///                      (unchanged across all pages of the same cursor).
    pub fn get_history_page_with_cursor(
        env: Env,
        player_id: u64,
        cursor_snapshot: Option<u32>,
        cursor_next_index: Option<u32>,
        limit: u32,
    ) -> (Vec<ProgressEntry>, u32, u32) {
        const MAX_PAGE: u32 = 50;

        // On the first call snapshot the current count so it never changes
        // for this logical cursor, even if advance_level is called concurrently.
        let snapshot_count: u32 = match cursor_snapshot {
            Some(s) => s,
            None => {
                let c: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::HistoryCounter(player_id))
                    .unwrap_or(0u32);
                // Read-side keep-alive (issue #1191): refresh the counter TTL on
                // the first call of a cursor pass so history browsed only through
                // this path isn't archived.
                if c > 0 {
                    env.storage().persistent().extend_ttl(
                        &DataKey::HistoryCounter(player_id),
                        PERSISTENT_TTL_MIN,
                        PERSISTENT_TTL_MAX,
                    );
                }
                c
            }
        };

        let next_index: u32 = cursor_next_index.unwrap_or(1);

        // All pages consumed (or empty history).
        if snapshot_count == 0 || next_index == 0 || next_index > snapshot_count {
            return (Vec::new(&env), 0u32, snapshot_count);
        }

        let effective_limit = limit.clamp(1, MAX_PAGE);
        let end = (next_index + effective_limit - 1).min(snapshot_count);

        let mut entries: Vec<ProgressEntry> = Vec::new(&env);
        for i in next_index..=end {
            let key = DataKey::HistoryEntry(player_id, i);
            if let Some(entry) = env.storage().persistent().get(&key) {
                env.storage().persistent().extend_ttl(
                    &key,
                    PERSISTENT_TTL_MIN,
                    PERSISTENT_TTL_MAX,
                );
                entries.push_back(entry);
            }
        }

        // next_index for the following page; 0 signals exhaustion.
        let returned_next = if end >= snapshot_count { 0u32 } else { end + 1 };

        (entries, returned_next, snapshot_count)
    }

    /// Query history entries for a player since a given Unix timestamp.
    /// Returns all entries where `updated_at >= since_timestamp`.
    /// Rebuilds the logical history from fixed-size `HistoryPage` shards so the
    /// query remains bounded even as the player's history grows.
    pub fn get_history_since(env: Env, player_id: u64, since_timestamp: u64) -> Vec<ProgressEntry> {
        let history = Self::read_history_pages(&env, player_id);
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

    /// Return the current Merkle commitment root over a player's full
    /// progress history — see `record_progress_entry`'s doc comment for how
    /// it is constructed and maintained.
    ///
    /// This is the value `verify_history_proof` checks proofs against. A
    /// caller who does not trust the RPC node serving this query can compare
    /// the root returned by multiple independent nodes, or re-derive it
    /// themselves from `get_progress_history` using the same construction.
    ///
    /// Returns 32 zero bytes for a player with no recorded history (mirrors
    /// the zero-value defaults used by `get_level` / `get_history_count` for
    /// unknown player IDs) — this is not a valid commitment for any real
    /// history and `verify_history_proof` never treats it as one, since it
    /// returns `PlayerNotFound` before comparing against it.
    pub fn get_progress_root(env: Env, player_id: u64) -> BytesN<32> {
        let key = DataKey::HistoryRoot(player_id);
        let root: BytesN<32> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| BytesN::from_array(&env, &[0u8; 32]));
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        root
    }

    /// Generate a Merkle inclusion proof for the history entry at `index`
    /// (1-indexed, matching `get_history_entry`) that verifies against the
    /// player's *current* `get_progress_root`.
    ///
    /// This is a read-only convenience for callers who do not want to
    /// re-implement the tree construction off-chain (an indexer, a test, a
    /// dispute-resolution UI); it recomputes the proof on demand from
    /// `HistoryVec` rather than storing it, since storing a proof per entry
    /// would require rewriting every prior entry's proof on each append.
    /// `verify_history_proof` does not depend on this function — it accepts
    /// any structurally valid proof from any source.
    pub fn get_history_proof(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<Vec<HistoryProofStep>, ProgressError> {
        let history = Self::read_history_pages(&env, player_id);
        let n = history.len();
        if n == 0 || index == 0 || index > n {
            return Err(ProgressError::PlayerNotFound);
        }

        let leaves = Self::leaf_hashes(&env, &history);
        let mut proof: Vec<HistoryProofStep> = Vec::new(&env);
        Self::path_range(&env, &leaves, index - 1, 0, n, &mut proof);
        Ok(proof)
    }

    /// Verify that `entry` is genuinely committed in `player_id`'s history at
    /// the *current* `get_progress_root`, using a caller-supplied Merkle
    /// proof.
    ///
    /// This is the independently-checkable half of the "tamper-proof
    /// history" guarantee: a light client, off-chain indexer, or dispute
    /// process can call this against any Soroban RPC node — including ones
    /// it does not otherwise trust — because the verification is a pure
    /// function of `(player_id, entry, proof, stored_root)` computed
    /// entirely on-chain, not an assertion the node makes about its own
    /// data.
    ///
    /// Returns `Ok(false)` — never panics — for a forged entry, a proof
    /// against a stale root (e.g. one predating the player's most recent
    /// append), or a structurally malformed proof (wrong length, empty when
    /// non-empty is required, garbage sibling bytes). Proofs longer than
    /// `MAX_PROOF_STEPS` are rejected as malformed without being hashed, so
    /// an adversarial caller cannot force unbounded verification cost by
    /// submitting an arbitrarily long proof Vec — the real proof depth for
    /// any history this contract can produce is `ceil(log2(n))`, and
    /// `MAX_PROOF_STEPS` leaves generous headroom above that.
    ///
    /// Returns `Err(ProgressError::PlayerNotFound)` only when the player has
    /// no committed root at all (no history has ever been recorded) — there
    /// is nothing to verify against, which is a different condition from an
    /// existing player's proof failing to verify.
    pub fn verify_history_proof(
        env: Env,
        player_id: u64,
        entry: ProgressEntry,
        proof: Vec<HistoryProofStep>,
    ) -> Result<bool, ProgressError> {
        // Real proof depth never exceeds ~32 even for a history no realistic
        // caller could ever grow (2^32 entries); this exists purely to bound
        // adversarial-input cost, not to constrain legitimate proofs.
        const MAX_PROOF_STEPS: u32 = 32;

        let root_key = DataKey::HistoryRoot(player_id);
        let stored_root: BytesN<32> = env
            .storage()
            .persistent()
            .get(&root_key)
            .ok_or(ProgressError::PlayerNotFound)?;
        env.storage()
            .persistent()
            .extend_ttl(&root_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        if proof.len() > MAX_PROOF_STEPS {
            return Ok(false);
        }
        // A proof cannot be replayed against a different player_id even if
        // its hash bytes happened to verify — the leaf hash binds player_id.
        if entry.player_id != player_id {
            return Ok(false);
        }

        let computed = Self::compute_root_from_proof(&env, &entry, &proof);
        Ok(computed == stored_root)
    }

    // -------------------------------------------------------------------------
    // Migration window management
    // -------------------------------------------------------------------------

    /// Open the one-time migration window.  Admin-only.
    ///
    /// While the window is open, `admin_seed_history` may be called to replay
    /// historical `ProgressEntry` records from an old contract deployment.
    /// Close the window with `close_migration_window` once replay is complete.
    ///
    /// **Security model**: the window must be opened *before* the new contract
    /// is exposed to live traffic, and closed *immediately* after replay.
    /// Leaving the window open permanently would allow the admin to fabricate
    /// arbitrary historical records post-launch.  The window flag is stored in
    /// instance storage so it is visible in `health()` and readable by any
    /// monitoring tool without a TTL concern.
    pub fn open_migration_window(env: Env) -> Result<(), ProgressError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MigrationActive, &true);
        Ok(())
    }

    /// Close the migration window.  Admin-only.
    ///
    /// Once closed, all `admin_seed_*` calls are rejected with
    /// `MigrationNotActive`.  This is the irreversibility gate: once the
    /// new contract is live, no further historical state can be injected.
    pub fn close_migration_window(env: Env) -> Result<(), ProgressError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MigrationActive, &false);
        Ok(())
    }

    /// Returns `true` if the migration window is currently open.
    pub fn migration_window_is_open(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::MigrationActive)
            .unwrap_or(false)
    }

        let pause_result = client.try_pause_contract();
        // Old admin no longer has permission
        assert!(pause_result.is_err());
    // -------------------------------------------------------------------------
    // Migration seeding
    // -------------------------------------------------------------------------

    /// Seed a historical `ProgressEntry` from a prior contract deployment.
    ///
    /// This is the migration entrypoint for **progress history** (MIGRATION_GAPS
    /// row 5).  It reconstructs the exact on-chain storage shape that
    /// `record_progress_entry` writes:
    ///
    /// - `HistoryEntry(player_id, history_index)` — the individual entry
    /// - `HistoryCounter(player_id)` — per-player monotonic counter
    /// - `HistoryVec(player_id)` — full Vec for O(1) reads
    /// - `HistoryRoot(player_id)` — RFC 6962 Merkle commitment root
    ///
    /// ## Idempotency
    ///
    /// Keyed on `(player_id, history_index)`.  If the entry already exists
    /// with byte-identical content the call is a **no-op** (returns `Ok(())`).
    /// If the key exists with *different* content the call returns
    /// `HistoryAlreadyExists` — conflicting rewrites are always rejected.
    ///
    /// ## Ordering
    ///
    /// `history_index` is 1-based and must equal `HistoryCounter + 1` at call
    /// time.  Out-of-order seeding (gap or duplicate at wrong position) returns
    /// `InvalidHistoryIndex`.  Callers must replay entries in ascending order.
    ///
    /// ## Merkle verification
    ///
    /// When `expected_root` is `Some`, after writing the entry this function
    /// recomputes the full Merkle root over all entries in `HistoryVec` using
    /// the same `mth_range` / `leaf_hash` logic as the live contract.  If the
    /// recomputed root ≠ `expected_root`, the function returns
    /// `MerkleRootMismatch`.  Soroban's transaction atomicity guarantees that
    /// **all writes are rolled back** on error, so no partial state persists.
    ///
    /// Supply `expected_root` on the *final* seed call for a player to perform
    /// an end-to-end integrity check.  Intermediate calls may pass `None`.
    ///
    /// ## Security
    ///
    /// Admin-only.  Requires the migration window to be open.  Does not invoke
    /// `advance_level` or any cross-contract call.  Does not validate that
    /// level transitions are logically valid — historical data is replayed as-is.
    pub fn admin_seed_history(
        env: Env,
        player_id: u64,
        history_index: u32,
        entry: ProgressEntry,
        expected_root: Option<BytesN<32>>,
    ) -> Result<(), ProgressError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        // history_index is 1-based; 0 is never a valid index.
        if history_index == 0 {
            return Err(ProgressError::InvalidHistoryIndex);
        }

        let entry_key = DataKey::HistoryEntry(player_id, history_index);

        // ── Idempotency check ─────────────────────────────────────────────────
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, ProgressEntry>(&entry_key)
        {
            let identical = existing.player_id == entry.player_id
                && existing.old_level == entry.old_level
                && existing.new_level == entry.new_level
                && existing.updated_by == entry.updated_by
                && existing.updated_at == entry.updated_at
                && existing.milestone_ref == entry.milestone_ref
                && existing.ledger_sequence == entry.ledger_sequence;

            if identical {
                // Idempotent replay: no-op.  Still run root verification if
                // the caller supplied expected_root so a retried final call
                // still validates correctness.
                if let Some(expected) = expected_root {
                    return Self::verify_and_seal_root(&env, player_id, &expected);
                }
                return Ok(());
            }
            // Conflicting content — reject without writing anything.
            return Err(ProgressError::HistoryAlreadyExists);
        }

        // ── Index continuity check ────────────────────────────────────────────
        let counter_key = DataKey::HistoryCounter(player_id);
        let current_counter: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0u32);
        let expected_next =
            safe_add_u32(current_counter, 1).map_err(|_| ProgressError::Overflow)?;
        if history_index != expected_next {
            return Err(ProgressError::InvalidHistoryIndex);
        }

        // ── Write HistoryEntry ────────────────────────────────────────────────
        env.storage().persistent().set(&entry_key, &entry);
        env.storage()
            .persistent()
            .extend_ttl(&entry_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Update HistoryCounter ─────────────────────────────────────────────
        env.storage().persistent().set(&counter_key, &history_index);
        env.storage()
            .persistent()
            .extend_ttl(&counter_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Append to HistoryVec ──────────────────────────────────────────────
        let vec_key = DataKey::HistoryVec(player_id);
        let mut history: Vec<ProgressEntry> = env
            .storage()
            .persistent()
            .get(&vec_key)
            .unwrap_or_else(|| Vec::new(&env));
        history.push_back(entry);
        env.storage().persistent().set(&vec_key, &history);
        env.storage()
            .persistent()
            .extend_ttl(&vec_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Recompute and persist Merkle root ─────────────────────────────────
        let root_key = DataKey::HistoryRoot(player_id);
        let leaves = Self::leaf_hashes(&env, &history);
        let root = Self::mth_range(&env, &leaves, 0, leaves.len());
        env.storage().persistent().set(&root_key, &root);
        env.storage()
            .persistent()
            .extend_ttl(&root_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Merkle root verification ──────────────────────────────────────────
        // If the caller supplied an expected root, compare it against the root
        // we just independently recomputed from the seeded history.
        // A mismatch means the replayed history is inconsistent with the
        // original commitment.  Soroban's atomicity rolls back all writes.
        if let Some(expected) = expected_root {
            if root != expected {
                return Err(ProgressError::MerkleRootMismatch);
            }
        }

        Ok(())
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
            pay_to_contact_paused: false,
        }
    }

    /// Returns the deployed crate version (from Cargo.toml at build time).
    pub fn version(env: Env) -> String {
        String::from_str(&env, CONTRACT_VERSION)
    }

    /// Returns a snapshot of all cross-contract peer address pointers held by
    /// this contract. Use this to verify that wiring is complete before
    /// relying on `advance_level`.
    ///
    /// This is a **read-only** function — it does not require auth, does not
    /// modify state, and is intentionally exempt from the pause/init guards
    /// so it remains callable even on a mis-wired or paused contract (that is
    /// exactly when you need it most).
    ///
    /// See `docs/WIRING_REGISTRY_DESIGN.md` for the full design context and
    /// the recommended migration path for already-deployed contracts.
    pub fn get_wiring_state(env: Env) -> ProgressWiringState {
        Self::bump_instance_ttl(&env);
        let registration_contract = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegistrationContract);
        let verification_contract = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::VerificationContract);
        let scout_access_contract = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ScoutAccessContract);
        let registration_epoch = env
            .storage()
            .instance()
            .get::<DataKey, u32>(&DataKey::RegistrationContractEpoch)
            .unwrap_or(0);
        let verification_epoch = env
            .storage()
            .instance()
            .get::<DataKey, u32>(&DataKey::VerificationContractEpoch)
            .unwrap_or(0);
        let scout_access_epoch = env
            .storage()
            .instance()
            .get::<DataKey, u32>(&DataKey::ScoutAccessContractEpoch)
            .unwrap_or(0);
        ProgressWiringState {
            registration_contract,
            verification_contract,
            scout_access_contract,
            registration_epoch,
            verification_epoch,
            scout_access_epoch,
        }
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

    fn history_page_index(index: u32) -> u32 {
        (index.saturating_sub(1)) / HISTORY_PAGE_SIZE
    }

    fn read_history_pages(env: &Env, player_id: u64) -> Vec<ProgressEntry> {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::HistoryCounter(player_id))
            .unwrap_or(0u32);

        if count == 0 {
            return env
                .storage()
                .persistent()
                .get(&DataKey::HistoryVec(player_id))
                .unwrap_or_else(|| Vec::new(env));
        }

        let total_pages = (count + HISTORY_PAGE_SIZE - 1) / HISTORY_PAGE_SIZE;
        let mut history: Vec<ProgressEntry> = Vec::new(env);
        for page_index in 0..total_pages {
            let page_key = DataKey::HistoryPage(player_id, page_index);
            let page: Vec<ProgressEntry> = env
                .storage()
                .persistent()
                .get(&page_key)
                .unwrap_or_else(|| {
                    let start = page_index * HISTORY_PAGE_SIZE + 1;
                    let end = (start + HISTORY_PAGE_SIZE - 1).min(count);
                    let mut reconstructed: Vec<ProgressEntry> = Vec::new(env);
                    for idx in start..=end {
                        if let Some(entry) = env
                            .storage()
                            .persistent()
                            .get(&DataKey::HistoryEntry(player_id, idx))
                        {
                            reconstructed.push_back(entry);
                        }
                    }
                    reconstructed
                });
            for i in 0..page.len() {
                if let Some(entry) = page.get(i) {
                    history.push_back(entry);
                }
            }
        }

        if history.is_empty() {
            env.storage()
                .persistent()
                .get(&DataKey::HistoryVec(player_id))
                .unwrap_or_else(|| Vec::new(env))
        } else {
            history
        }
    }

    /// Numeric tier code for a `ProgressLevel`, used only for canonical leaf
    /// serialization in `leaf_hash`. Not derived from the enum's Rust
    /// discriminant (which is not part of any stability contract) so the
    /// commitment scheme's byte layout stays fixed even if variant order in
    /// `ProgressLevel` is ever reshuffled.
    fn level_code(level: &ProgressLevel) -> u32 {
        match level {
            ProgressLevel::Unverified => 0,
            ProgressLevel::VerifiedIdentity => 1,
            ProgressLevel::PerformanceMilestones => 2,
            ProgressLevel::EliteTier => 3,
        }
    }

    /// Canonical leaf hash for one `ProgressEntry`, per the RFC 6962 Merkle
    /// Tree Hash convention: `H(0x00 || <canonical field bytes>)`. The
    /// `0x00` domain-separates leaf hashes from internal-node hashes
    /// (`node_hash`'s `0x01` prefix) so a leaf can never be replayed as an
    /// internal node or vice versa (the classic second-preimage attack
    /// against naive Merkle trees).
    ///
    /// Field order is fixed: player_id, old_level, new_level, updated_by,
    /// updated_at, milestone_ref, ledger_sequence — matching `ProgressEntry`'s
    /// declaration order. `updated_by` is serialized via `to_xdr`, Soroban's
    /// canonical `Address` encoding, rather than any string form.
    fn leaf_hash(env: &Env, entry: &ProgressEntry) -> BytesN<32> {
        let mut b = Bytes::new(env);
        b.push_back(0u8);
        b.extend_from_slice(&entry.player_id.to_be_bytes());
        b.extend_from_slice(&Self::level_code(&entry.old_level).to_be_bytes());
        b.extend_from_slice(&Self::level_code(&entry.new_level).to_be_bytes());
        b.append(&entry.updated_by.clone().to_xdr(env));
        b.extend_from_slice(&entry.updated_at.to_be_bytes());
        b.extend_from_slice(&entry.milestone_ref.to_be_bytes());
        b.extend_from_slice(&entry.ledger_sequence.to_be_bytes());
        env.crypto().sha256(&b).to_bytes()
    }

    /// Canonical internal-node hash: `H(0x01 || left || right)`. See
    /// `leaf_hash` for why the `0x01` prefix (domain separation) matters.
    fn node_hash(env: &Env, left: &BytesN<32>, right: &BytesN<32>) -> BytesN<32> {
        let mut b = Bytes::new(env);
        b.push_back(1u8);
        b.append(&Bytes::from_slice(env, &left.to_array()));
        b.append(&Bytes::from_slice(env, &right.to_array()));
        env.crypto().sha256(&b).to_bytes()
    }

    fn leaf_hashes(env: &Env, history: &Vec<ProgressEntry>) -> Vec<BytesN<32>> {
        let mut leaves: Vec<BytesN<32>> = Vec::new(env);
        for i in 0..history.len() {
            let e = history.get(i).unwrap();
            leaves.push_back(Self::leaf_hash(env, &e));
        }
        leaves
    }

    /// Largest power of two strictly less than `n` (`n` must be `>= 2`).
    /// This is the split point RFC 6962's Merkle Tree Hash uses to divide an
    /// arbitrary-size leaf range into a perfect left subtree and a
    /// (possibly imperfect) right subtree — the standard, formally
    /// specified way to build a deterministic binary Merkle tree over any
    /// number of leaves, not just powers of two.
    fn largest_pow2_lt(n: u32) -> u32 {
        let mut k: u32 = 1;
        while k.saturating_mul(2) < n {
            k = k.saturating_mul(2);
        }
        k
    }

    /// RFC 6962 Merkle Tree Hash (MTH) of `leaves[start..end]`.
    ///
    /// Recomputed from scratch on every call rather than maintained
    /// incrementally (e.g. via an MMR peaks accumulator): this contract's
    /// `record_progress_entry` already reads and rewrites the player's full
    /// `HistoryVec` on every append for an unrelated, pre-existing reason
    /// (`get_progress_history`'s O(1)-read optimization — see that key's
    /// doc comment), so the leaf list is already fully materialized in
    /// memory at zero extra storage cost. Recomputing `O(n)` leaf/node
    /// hashes on top of that is `O(n)` extra CPU with no extra storage I/O,
    /// which is cheaper than the storage cost an incremental accumulator
    /// would need to persist and is simpler to get correct — and `n` is
    /// bounded in practice to a handful of entries (three tier advances,
    /// plus any admin dispute-resolution resets via `reset_player_level`),
    /// not an unbounded log. See `ci/cpu-cost-budget.md` for the measured
    /// cost this adds to `advance_level`.
    fn mth_range(env: &Env, leaves: &Vec<BytesN<32>>, start: u32, end: u32) -> BytesN<32> {
        let n = end - start;
        if n == 1 {
            return leaves.get(start).unwrap();
        }
        let k = Self::largest_pow2_lt(n);
        let left = Self::mth_range(env, leaves, start, start + k);
        let right = Self::mth_range(env, leaves, start + k, end);
        Self::node_hash(env, &left, &right)
    }

    /// RFC 6962 audit path (`PATH`) for leaf `index` within `leaves[start..end]`,
    /// appended into `proof` in leaf-to-root order (the order
    /// `compute_root_from_proof` expects to replay). `index` is an absolute
    /// position into the full `leaves` Vec, not relative to `start`.
    fn path_range(
        env: &Env,
        leaves: &Vec<BytesN<32>>,
        index: u32,
        start: u32,
        end: u32,
        proof: &mut Vec<HistoryProofStep>,
    ) {
        let n = end - start;
        if n == 1 {
            return;
        }
        let k = Self::largest_pow2_lt(n);
        if index - start < k {
            Self::path_range(env, leaves, index, start, start + k, proof);
            let right_root = Self::mth_range(env, leaves, start + k, end);
            proof.push_back(HistoryProofStep {
                sibling: right_root,
                sibling_is_right: true,
            });
        } else {
            Self::path_range(env, leaves, index, start + k, end, proof);
            let left_root = Self::mth_range(env, leaves, start, start + k);
            proof.push_back(HistoryProofStep {
                sibling: left_root,
                sibling_is_right: false,
            });
        }
    }

    /// Replay a proof against `entry`'s leaf hash to recompute the root it
    /// implies. Never panics on a malformed `proof`: any `HistoryProofStep`
    /// sequence — of any length, containing any bytes — deterministically
    /// hashes to *some* 32-byte value, which simply will not equal the
    /// stored root unless the proof is genuine.
    fn compute_root_from_proof(
        env: &Env,
        entry: &ProgressEntry,
        proof: &Vec<HistoryProofStep>,
    ) -> BytesN<32> {
        let mut current = Self::leaf_hash(env, entry);
        for i in 0..proof.len() {
            let step = proof.get(i).unwrap();
            if step.sibling_is_right {
                current = Self::node_hash(env, &current, &step.sibling);
            } else {
                current = Self::node_hash(env, &step.sibling, &current);
            }
        }
        current
    }

    /// Record a progress entry for a player.
    ///
    /// ## Storage cost trade-off (HistoryCounter)
    ///
    /// Each call performs a read + write on `DataKey::HistoryCounter(player_id)`.
    /// On Soroban, persistent writes are the most expensive storage operation.
    /// For high-frequency players this is two storage ops per call.
    ///
    /// **Current approach (separate counter key):**
    /// - Simple, O(1) counter read for `get_history_count`.
    /// - Two storage ops per `advance_level` call (read + write counter).
    ///
    /// **Alternative A — inline counter in HistoryVec:**
    /// Store the count as `history.len()`. Eliminates the separate counter key
    /// entirely, saving one persistent read + write per call. However,
    /// `get_history_count` would require loading the full Vec just to read
    /// its length, which becomes expensive as history grows.
    ///
    /// **Alternative B — batch accumulation:**
    /// If batch milestone approval is implemented, accumulate counter
    /// increments in memory and flush a single write at the end of the
    /// batch. This amortises the write cost across N milestones but adds
    /// complexity and is only beneficial when batch operations exist.
    ///
    /// **Decision:** Keep the current separate-counter approach for its
    /// simplicity and O(1) count queries. Revisit if batch milestone
    /// approval is implemented or if per-player milestone frequency
    /// exceeds ~10 per ledger close window.
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
        let next_index = safe_add_u32(index, 1).map_err(|_| ProgressError::Overflow)?;

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
        env.storage()
            .persistent()
            .extend_ttl(&history_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // Store the player's history in bounded pages instead of one ever-growing
        // `HistoryVec` key. Every page remains small and fixed-size, while the
        // logical history is reconstructed by concatenating the pages in order.
        let page_index = Self::history_page_index(next_index);
        let page_key = DataKey::HistoryPage(player_id, page_index);
        let mut page: Vec<ProgressEntry> = env
            .storage()
            .persistent()
            .get(&page_key)
            .unwrap_or_else(|| Vec::new(env));
        page.push_back(entry.clone());
        env.storage().persistent().set(&page_key, &page);
        env.storage()
            .persistent()
            .extend_ttl(&page_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // Recompute the Merkle commitment root over the full (now-updated)
        // logical history from the page shards. This preserves the existing
        // proof semantics while preventing a single persistent key from growing
        // without bound.
        let root_key = DataKey::HistoryRoot(player_id);
        let history = Self::read_history_pages(env, player_id);
        let leaves = Self::leaf_hashes(env, &history);
        let root = Self::mth_range(env, &leaves, 0, leaves.len());
        env.storage().persistent().set(&root_key, &root);
        env.storage()
            .persistent()
            .extend_ttl(&root_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        Ok(())
    }

    /// Require the migration window to be open.
    fn require_migration_active(env: &Env) -> Result<(), ProgressError> {
        let active = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::MigrationActive)
            .unwrap_or(false);
        if !active {
            return Err(ProgressError::MigrationNotActive);
        }
        Ok(())
    }

    /// Verify the persisted `HistoryRoot` for `player_id` matches `expected`.
    ///
    /// Used in idempotent replay paths where the root was already committed by
    /// a prior identical call.  Avoids a full re-hash on pure no-op retries.
    fn verify_and_seal_root(
        env: &Env,
        player_id: u64,
        expected: &BytesN<32>,
    ) -> Result<(), ProgressError> {
        let root_key = DataKey::HistoryRoot(player_id);
        let existing_root: BytesN<32> = env
            .storage()
            .persistent()
            .get(&root_key)
            .ok_or(ProgressError::PlayerNotFound)?;
        if &existing_root != expected {
            return Err(ProgressError::MerkleRootMismatch);
        }
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
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke},
        vec, Env, IntoVal, Symbol,
    };

    #[test]
    fn test_get_verification_contract_before_and_after_configuration() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &id);

        assert_eq!(client.get_verification_contract(), None);

        client.initialize(&Address::generate(&env));
        let peer = Address::generate(&env);
        client.set_verification_contract(&peer);
        assert_eq!(client.get_verification_contract(), Some(peer));
    }

    #[test]
    fn test_get_registration_contract_before_and_after_configuration() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &id);

        assert_eq!(client.get_registration_contract(), None);

        client.initialize(&Address::generate(&env));
        let peer = Address::generate(&env);
        client.set_registration_contract(&peer);
        assert_eq!(client.get_registration_contract(), Some(peer));
    }

    #[test]
    fn test_get_scout_access_contract_before_and_after_configuration() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &id);

        assert_eq!(client.get_scout_access_contract(), None);

        client.initialize(&Address::generate(&env));
        let peer = Address::generate(&env);
        client.set_scout_access_contract(&peer);
        assert_eq!(client.get_scout_access_contract(), Some(peer));
    }

    /// Deterministically generate a syntactically valid CIDv0 string (46 chars,
    /// "Qm" prefix, base58btc charset) so tests can approve unique milestones.
    fn dummy_cid(env: &Env, seed: u32) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut buf = [0u8; 46];
        buf[0] = b'Q';
        buf[1] = b'm';
        let mut x = seed;
        for slot in buf.iter_mut().skip(2) {
            *slot = ALPHABET[(x as usize) % ALPHABET.len()];
            x = x.wrapping_mul(31).wrapping_add(7);
        }
        String::from_str(env, core::str::from_utf8(&buf).unwrap())
    }

    fn setup() -> (Env, ProgressContractClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Wire a real verification contract so advance_level's on-chain
        // milestone_ref validation (#457) has something to query. Pre-approve
        // milestones 1-10 (via two validators, since MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR
        // caps each validator at 5) for every player_id used across this test
        // suite so existing level-progression tests (unrelated to milestone
        // validation itself) keep passing.
        let ver_id = env.register(scoutchain_verification::VerificationContract, ());
        let ver_client = scoutchain_verification::VerificationContractClient::new(&env, &ver_id);
        let ver_admin = Address::generate(&env);
        ver_client.initialize(&ver_admin);
        let mut cid_seed: u32 = 0;
        for player_id in [1u64, 2, 5, 7, 10, 20, 42, 55] {
            for _ in 0..2u32 {
                let milestone_validator = Address::generate(&env);
                ver_client.register_validator(&milestone_validator, &String::from_str(&env, "Test License"), &String::from_str(&env, "Test Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::vec![&env]);
                for _ in 0..5 {
                    cid_seed += 1;
                    ver_client.approve_milestone(
                        &milestone_validator,
                        &player_id,
                        &String::from_str(&env, "test milestone"),
                        &dummy_cid(&env, cid_seed),
                        &None,
                    );
                }
            }
        }
        client.set_verification_contract(&ver_id);

        // Caller identity used across tests to invoke advance_level. Auth is
        // mocked in these tests, so its identity need not match the validator
        // registered on the verification contract above.
        let validator = Address::generate(&env);
        (env, client, validator)
    }

    #[test]
    fn test_two_players_advance_independently() {
        let (_env, client, validator) = setup();

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
        use soroban_sdk::testutils::Ledger;
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

    // #1191: history read ONLY through get_progress_history_page must stay
    // accessible across a long ledger span. The read path must extend the TTL
    // of the HistoryCounter and every HistoryEntry it touches; otherwise the
    // write-side extension decays and the history is archived while a UI is
    // still actively browsing it.
    #[test]
    fn test_get_progress_history_page_keeps_history_alive() {
        use soroban_sdk::testutils::Ledger;
        let (env, client, validator) = setup();

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        // Write three history entries; each write extends TTL to
        // 100_000 + PERSISTENT_TTL_MAX (= 618_400).
        let player_id = 55u64;
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        // Move partway through the write-side TTL and read ONLY via the
        // paginated getter — this must re-extend the history keys.
        env.ledger().with_mut(|l| l.sequence_number = 500_000);
        let page = client.get_progress_history_page(&player_id, &0u32, &50u32);
        assert_eq!(page.len(), 3);

        // Advance past the original write-side TTL (618_400). Without the
        // read-side extension the counter/entries would now be archived.
        env.ledger().with_mut(|l| l.sequence_number = 900_000);
        let page = client.get_progress_history_page(&player_id, &0u32, &50u32);
        assert_eq!(
            page.len(),
            3,
            "history read only via the paginated getter must stay accessible"
        );
    }

    // #1191: same keep-alive guarantee for the cursor-based paginated getter.
    #[test]
    fn test_get_history_page_with_cursor_keeps_history_alive() {
        use soroban_sdk::testutils::Ledger;
        let (env, client, validator) = setup();

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        let player_id = 55u64;
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        env.ledger().with_mut(|l| l.sequence_number = 500_000);
        let (entries, _next, snapshot) =
            client.get_history_page_with_cursor(&player_id, &None, &None, &50u32);
        assert_eq!(entries.len(), 3);
        assert_eq!(snapshot, 3);

        env.ledger().with_mut(|l| l.sequence_number = 900_000);
        let (entries, _next, snapshot) =
            client.get_history_page_with_cursor(&player_id, &None, &None, &50u32);
        assert_eq!(
            entries.len(),
            3,
            "cursor-paginated history must stay accessible across a long span"
        );
        assert_eq!(snapshot, 3);
    }

    // PlayerLevel TTL must be extended when reset_player_level writes it —
    // otherwise the reset level silently reverts to Unverified (get_level's
    // default) once the un-bumped entry expires.
    #[test]
    fn test_reset_player_level_ttl_extended_after_write() {
        use soroban_sdk::testutils::Ledger;
        let (env, client, _validator) = setup();

        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000;
            l.min_persistent_entry_ttl = 500;
            l.max_entry_ttl = 600_000;
        });

        let player_id = 55u64;
        client.reset_player_level(&player_id, &ProgressLevel::EliteTier);

        // Advance ledger past what the default un-bumped TTL would be.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000 + 1_000;
        });

        // The reset level must still be readable — TTL was extended on write.
        // Without the fix, this would fall back to ProgressLevel::Unverified.
        assert_eq!(client.get_level(&player_id), ProgressLevel::EliteTier);
    }

    #[test]
    fn test_advance_level_not_initialized() {
        let env = Env::default();
        env.mock_all_auths();
        // Register the contract but deliberately skip initialize()
        let id = env.register(ProgressContract, ());
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
        let id = env.register(ProgressContract, ());
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

    // #397: With DataKey::VerificationContract configured (done in setup()),
    // advance_level must reject a direct call from any address that is
    // neither the configured VerificationContract nor the optional
    // ScoutAccessContract — the caller whitelist must not have an open
    // fallback.
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
        let (_env, client, validator) = setup();
        let player_id = 20u64;

        // Advance through all 3 tiers
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);

        // First page: offset=0, limit=2 → entries 1,2
        let page1 = client.get_progress_history_page(&player_id, &0u32, &2u32);
        assert_eq!(page1.len(), 2);
        assert_eq!(page1.get(0).unwrap().old_level, ProgressLevel::Unverified);
        assert_eq!(
            page1.get(1).unwrap().old_level,
            ProgressLevel::VerifiedIdentity
        );

        // Middle page: offset=1, limit=1 → entry 2
        let mid = client.get_progress_history_page(&player_id, &1u32, &1u32);
        assert_eq!(mid.len(), 1);
        assert_eq!(
            mid.get(0).unwrap().old_level,
            ProgressLevel::VerifiedIdentity
        );

        // Last page: offset=2, limit=50 → entry 3 only
        let last = client.get_progress_history_page(&player_id, &2u32, &50u32);
        assert_eq!(last.len(), 1);
        assert_eq!(last.get(0).unwrap().new_level, ProgressLevel::EliteTier);

        // A zero limit is floored at one and still returns the first entry.
        let zero_limit = client.get_progress_history_page(&player_id, &0u32, &0u32);
        assert_eq!(zero_limit.len(), 1);
        assert_eq!(
            zero_limit.get(0).unwrap().old_level,
            ProgressLevel::Unverified
        );

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
                        Symbol::new(&env, crate::events::PROGRESS_UPDATED).into_val(&env),
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
    fn test_admin_transfer_propose_replace_and_accept() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &contract_id);
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
                    contract_id.clone(),
                    vec![
                        &env,
                        Symbol::new(&env, events::ADMIN_TRANSFER_PROPOSED).into_val(&env),
                        old_admin.clone().into_val(&env),
                    ],
                    stale_admin.clone().into_val(&env),
                )
            ]
        );

        // The old admin remains fully functional while acceptance is pending.
        client.pause_contract();
        client.unpause_contract();

        client.propose_admin(&new_admin);
        env.as_contract(&contract_id, || {
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
                contract: &contract_id,
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
                    contract_id.clone(),
                    vec![
                        &env,
                        Symbol::new(&env, events::ADMIN_TRANSFERRED).into_val(&env),
                        old_admin.clone().into_val(&env),
                    ],
                    new_admin.clone().into_val(&env),
                )
            ]
        );
        env.as_contract(&contract_id, || {
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
    fn test_transfer_admin_alias_creates_proposal() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &contract_id);
        let old_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        client.initialize(&old_admin);
        client.transfer_admin(&new_admin);

        env.as_contract(&contract_id, || {
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::Admin),
                Some(old_admin)
            );
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::PendingAdmin),
                Some(new_admin)
            );
        });
    }

    // #396: A non-admin caller must not be able to transfer admin control.
    // `transfer_admin` (an alias for `propose_admin`) is gated by the shared
    // `require_admin` helper, which calls `Address::require_auth()` on the
    // *stored* admin address. When that auth isn't satisfied, the SDK
    // surfaces it as a host-level auth error on `try_transfer_admin` (caught
    // here without panicking, matching the pattern used by
    // `test_advance_level_unauthorized_when_verification_contract_set`)
    // rather than a decoded `ProgressError::Unauthorized` contract error —
    // `require_admin` never constructs that variant, since auth failures are
    // rejected before contract logic runs. This test locks in that the call
    // fails end-to-end so a future refactor of the auth guard can't silently
    // let a non-admin caller through.
    #[test]
    fn test_transfer_admin_called_by_non_admin_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        client.initialize(&admin);

        // No authorizations are mocked for this call, so the stored admin's
        // `require_auth()` inside `require_admin` cannot be satisfied.
        env.mock_auths(&[]);
        let result = client.try_transfer_admin(&new_admin);
        assert!(result.is_err(), "non-admin caller must not transfer admin");

        // Admin must be unchanged and no pending proposal recorded.
        env.as_contract(&contract_id, || {
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<DataKey, Address>(&DataKey::Admin),
                Some(admin)
            );
            assert!(!env.storage().persistent().has(&DataKey::PendingAdmin));
        });
    }

    #[test]
    #[should_panic]
    fn test_third_party_cannot_accept_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &contract_id);
        let old_admin = Address::generate(&env);
        let pending_admin = Address::generate(&env);
        let third_party = Address::generate(&env);
        client.initialize(&old_admin);
        client.propose_admin(&pending_admin);

        env.mock_auths(&[MockAuth {
            address: &third_party,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
    }

    #[test]
    fn test_pause_and_unpause() {
        let (_env, client, validator) = setup();
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
    fn test_upgrade_preserves_admin() {
        let env = Env::default();
        env.mock_all_auths();

        let id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Wire a real verification contract with one approved milestone so
        // advance_level's on-chain milestone_ref validation (#457) succeeds.
        let ver_id = env.register(scoutchain_verification::VerificationContract, ());
        let ver_client = scoutchain_verification::VerificationContractClient::new(&env, &ver_id);
        let ver_admin = Address::generate(&env);
        ver_client.initialize(&ver_admin);
        let milestone_validator = Address::generate(&env);
        ver_client.register_validator(&milestone_validator, &String::from_str(&env, "Test License"), &String::from_str(&env, "Test Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::vec![&env]);
        let player_id = 1u64;
        ver_client.approve_milestone(
            &milestone_validator,
            &player_id,
            &String::from_str(&env, "test milestone"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
            &None,
        );
        client.set_verification_contract(&ver_id);

        client.advance_level(&milestone_validator, &player_id, &1u32);

        // Simulate upgrade: in testutils mode the host accepts empty bytes
        let new_wasm_hash = env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::new(&env));

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
        // Self-contained (rather than using the shared setup() helper) so
        // this test can assert the exact wiring_updated-free event shape
        // below against a known `admin` address. advance_level's on-chain
        // milestone_ref validation only applies to the secondary
        // (scout_access) caller path — the primary VerificationContract
        // caller (any address, once set_verification_contract is called and
        // auth is mocked) is trusted without a real deployed verification
        // contract, matching the pattern already used by e.g.
        // test_advance_level_sequence via setup().
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, ProgressContract);
        let client = ProgressContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let verification = Address::generate(&env);
        client.set_verification_contract(&verification);

        let validator = Address::generate(&env);
        let player_id = 1u64;

        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        assert_eq!(client.get_history_count(&player_id), 2);

        // Read the admin first: `as_contract` is itself an invocation, and
        // `events().all()` only reflects the most recent one, so doing this
        // after the reset would wipe the log we are about to assert on.
        let admin: Address = env.as_contract(&client.address, || {
            env.storage().persistent().get(&DataKey::Admin).unwrap()
        });

        client.reset_player_level(&player_id, &ProgressLevel::Unverified);

        // The event is still emitted — checked immediately, since `events().all()`
        // only reflects the most recent contract invocation and the read calls
        // below are themselves separate invocations.
        //
        // `ContractEvents` is an opaque handle, not a `Vec`: it exposes no
        // `len`/`get`/iteration, only equality against a
        // `Vec<(Address, Vec<Val>, Val)>`. So assert on the whole log at once,
        // the same idiom the other event tests in this module use. Topics are
        // (Symbol, admin); data is (player_id, old_level, target_level).
        assert_eq!(
            env.events().all(),
            vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, crate::events::PLAYER_LEVEL_RESET), admin,).into_val(&env),
                    (
                        player_id,
                        ProgressLevel::PerformanceMilestones,
                        ProgressLevel::Unverified,
                    )
                        .into_val(&env),
                )
            ]
        );

        assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);
        assert_eq!(client.get_history_count(&player_id), 3);

        let reset_entry = client.get_history_entry(&player_id, &3u32);
        assert_eq!(reset_entry.old_level, ProgressLevel::PerformanceMilestones);
        assert_eq!(reset_entry.new_level, ProgressLevel::Unverified);
        assert_eq!(reset_entry.milestone_ref, 0);
    }

    #[test]
    #[should_panic]
    fn test_reset_player_level_unauthorized() {
        let (env, client, _) = setup();
        env.mock_auths(&[]);
        client.reset_player_level(&1u64, &ProgressLevel::Unverified);
    }

    #[test]
    fn test_reset_player_level_rejects_same_level() {
        let (_, client, validator) = setup();
        let player_id = 1u64;

        // Advance to level 2 (PerformanceMilestones)
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        assert_eq!(client.get_level(&player_id), ProgressLevel::PerformanceMilestones);

        // Attempt to reset to the same level — should fail
        let result = client.try_reset_player_level(&player_id, &ProgressLevel::PerformanceMilestones);
        assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));
        
        // Level should remain unchanged
        assert_eq!(client.get_level(&player_id), ProgressLevel::PerformanceMilestones);
        // History should have only the 2 advances, no reset entry
        assert_eq!(client.get_history_count(&player_id), 2);
    }

    #[test]
    fn test_reset_player_level_rejects_forward_jump() {
        let (_, client, validator) = setup();
        let player_id = 1u64;

        // Advance to level 1 (VerifiedIdentity)
        client.advance_level(&validator, &player_id, &1u32);
        assert_eq!(client.get_level(&player_id), ProgressLevel::VerifiedIdentity);

        // Attempt to reset forward to level 3 (EliteTier) — should fail
        let result = client.try_reset_player_level(&player_id, &ProgressLevel::EliteTier);
        assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));
        
        // Level should remain at 1, not jump to 3
        assert_eq!(client.get_level(&player_id), ProgressLevel::VerifiedIdentity);
        // History should have only the 1 advance, no reset entry
        assert_eq!(client.get_history_count(&player_id), 1);
    }

    #[test]
    fn test_reset_player_level_allows_valid_rollback() {
        let (_, client, validator) = setup();
        let player_id = 1u64;

        // Advance to level 3 (EliteTier)
        client.advance_level(&validator, &player_id, &1u32);
        client.advance_level(&validator, &player_id, &2u32);
        client.advance_level(&validator, &player_id, &3u32);
        assert_eq!(client.get_level(&player_id), ProgressLevel::EliteTier);
        assert_eq!(client.get_history_count(&player_id), 3);

        // Reset down to level 1 (VerifiedIdentity) — should succeed
        let result = client.try_reset_player_level(&player_id, &ProgressLevel::VerifiedIdentity);
        assert_eq!(result, Ok(Ok(())));
        
        // Level should now be 1
        assert_eq!(client.get_level(&player_id), ProgressLevel::VerifiedIdentity);
        // History should have 3 advances + 1 reset entry
        assert_eq!(client.get_history_count(&player_id), 4);
        
        let reset_entry = client.get_history_entry(&player_id, &4u32);
        assert_eq!(reset_entry.old_level, ProgressLevel::EliteTier);
        assert_eq!(reset_entry.new_level, ProgressLevel::VerifiedIdentity);
        assert_eq!(reset_entry.milestone_ref, 0);
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

        // The emitted event must reflect the same new_level that is in storage.
        // Checked immediately — `events().all()` only reflects the most recent
        // invocation, and the reads below are themselves separate invocations.
        let events = env.events().all();
        assert_eq!(events.events().len(), 1);

        // After advance_level returns, both the storage state and the event
        // must agree: the player is at VerifiedIdentity.
        let stored_level = client.get_level(&player_id);
        assert_eq!(stored_level, ProgressLevel::VerifiedIdentity);
        // Event data encodes (player_id, old_level, new_level).
        // We verify new_level in storage equals VerifiedIdentity, which is
        // what the event carries — confirming the write happened before emit.
        let history = client.get_progress_history(&player_id);
        assert_eq!(history.get(0).unwrap().new_level, stored_level);
    }

    // -------------------------------------------------------------------------
    // #457: milestone_ref is validated against the verification contract
    // -------------------------------------------------------------------------

    // Milestone-ref validation only runs for the secondary (ScoutAccessContract)
    // caller. The primary VerificationContract caller is always invoked as a
    // nested call from its own `approve_milestone` (the only place it calls
    // advance_level), so calling back into it to re-validate the very
    // milestone_ref it just created would both be redundant and trigger a
    // disallowed contract re-entry. ScoutAccessContract, by contrast, is an
    // untrusted-for-milestone-data caller reachable via a clean (non-nested)
    // call, so validating its milestone_ref is both meaningful and safe.
    #[test]
    fn test_advance_level_invalid_milestone_ref_rejected_for_secondary_caller() {
        use scoutchain_verification::VerificationContract;
        use scoutchain_verification::VerificationContractClient;

        let env = Env::default();
        env.mock_all_auths();

        // Deploy verification contract and register a validator + milestone.
        let ver_id = env.register(VerificationContract, ());
        let ver_client = VerificationContractClient::new(&env, &ver_id);
        let ver_admin = Address::generate(&env);
        ver_client.initialize(&ver_admin);

        // Deploy progress contract and wire the verification + scout_access addresses.
        let prog_id = env.register(ProgressContract, ());
        let prog_client = ProgressContractClient::new(&env, &prog_id);
        let prog_admin = Address::generate(&env);
        prog_client.initialize(&prog_admin);
        prog_client.set_verification_contract(&ver_id);
        let scout_access = Address::generate(&env);
        prog_client.set_scout_access_contract(&scout_access);

        let validator = Address::generate(&env);
        ver_client.register_validator(&validator, &soroban_sdk::String::from_str(&env, "UEFA-B-License"), &soroban_sdk::String::from_str(&env, "Test Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::vec![&env]);
        // Approve one milestone for player 1 → milestone_ref 1 is valid.
        ver_client.approve_milestone(
            &validator,
            &1u64,
            &soroban_sdk::String::from_str(&env, "scored"),
            &soroban_sdk::String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
            &None,
        );

        // Valid ref (1) must succeed.
        let result = prog_client.try_advance_level(&scout_access, &1u64, &1u32);
        assert!(result.is_ok(), "valid milestone_ref should succeed");

        // Non-existent ref (99) must be rejected.
        let result = prog_client.try_advance_level(&scout_access, &1u64, &99u32);
        assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));

        // Zero ref must also be rejected.
        let result = prog_client.try_advance_level(&scout_access, &1u64, &0u32);
        assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));
    }

    #[test]
    fn test_advance_level_skips_milestone_validation_when_verification_not_set() {
        // When no VerificationContract is configured, any milestone_ref is
        // accepted (backward-compatible behaviour).
        let env = Env::default();
        env.mock_all_auths();
        let prog_id = env.register(ProgressContract, ());
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
        let prog_id2 = env2.register(ProgressContract, ());
        let prog_client2 = ProgressContractClient::new(&env2, &prog_id2);
        let admin2 = Address::generate(&env2);
        prog_client2.initialize(&admin2);
        // No set_verification_contract call here — should error NotInitialized
        // (no VerificationContract key means advance_level rejects).
        let caller = Address::generate(&env2);
        let result = prog_client2.try_advance_level(&caller, &1u64, &99u32);
        assert_eq!(result, Err(Ok(ProgressError::NotInitialized)));
    }

    // #398: get_level falls back to ProgressLevel::Unverified when no
    // PlayerLevel storage key exists yet (i.e. the player has never called
    // advance_level). Guards against a regression where removing the
    // unwrap_or default would panic instead of returning Unverified.
    #[test]
    fn test_get_level_returns_unverified_when_no_advance() {
        let (_, client, _) = setup();
        assert_eq!(client.get_level(&999u64), ProgressLevel::Unverified);
    }

    // #399: get_history_count falls back to 0 when no HistoryCounter storage
    // key exists yet (i.e. the player has no history). Guards against a
    // regression where removing the unwrap_or(0) default would panic on the
    // first query for any new player.
    #[test]
    fn test_get_history_count_returns_zero_when_no_progress() {
        let (_, client, _) = setup();
        assert_eq!(client.get_history_count(&999u64), 0);
    }

    #[test]
    fn test_pause_contract_emits_contract_paused_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let verification = Address::generate(&env);
        client.set_verification_contract(&verification);

        client.pause_contract();
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "contract_paused"), admin.clone()).into_val(&env),
                    ().into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_unpause_contract_emits_contract_unpaused_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ProgressContract, ());
        let client = ProgressContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let verification = Address::generate(&env);
        client.set_verification_contract(&verification);

        client.pause_contract();
        client.unpause_contract();

        // `events().all()` only reflects the most recent contract invocation,
        // so after `unpause_contract` the log holds exactly the unpause event.
        // `ContractEvents` is not an iterator and has no `last()`; it only
        // supports equality against a `Vec<(Address, Vec<Val>, Val)>`.
        assert_eq!(
            env.events().all(),
            vec![
                &env,
                (
                    client.address.clone(),
                    (Symbol::new(&env, "contract_unpaused"), admin.clone()).into_val(&env),
                    ().into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_version() {
        let (env, client, _) = setup();
        assert_eq!(
            client.version(),
            String::from_str(&env, env!("CARGO_PKG_VERSION"))
        );
    }

    // #705: Core test proving the archival failure and the fix.
    // Demonstrates that PlayerLevel records cannot be silently archived after
    // extended dormancy, and that get_level properly extends TTL on read.
    // This test will FAIL on unfixed code and PASS on fixed code.
    #[test]
    fn test_player_level_survives_extended_dormancy_via_ttl_extension() {
        use soroban_sdk::testutils::Ledger;

        // `setup()` already initializes the contract and wires a verification
        // contract; re-initializing would fail with AlreadyInitialized (#1).
        // Its third return value is the *caller* address used to invoke
        // advance_level — a plain generated address, not a deployed contract.
        let (env, client, caller) = setup();

        // Set a deterministic starting ledger sequence.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100;
            l.max_entry_ttl = 600_000; // Allow extended TTL values in the test
        });

        // Advance the player to the top tier. The enum's maximum is
        // `EliteTier` (there is no `Elite` variant), and each `advance_level`
        // moves exactly one tier, so reaching it takes three calls.
        //
        // These are ordinary client calls. Wrapping them in
        // `env.as_contract(&caller, ...)` would fail with Storage/MissingValue
        // ("non-existing value for contract instance"), because `caller` is a
        // generated address with no contract instance behind it. It is also
        // unnecessary: auth is mocked, and `advance_level` authorizes against
        // the *stored* whitelist address rather than the caller argument.
        client.advance_level(&caller, &1u64, &1u32);
        client.advance_level(&caller, &1u64, &2u32);
        client.advance_level(&caller, &1u64, &3u32);

        // Verify the player is now at the top tier
        assert_eq!(client.get_level(&1u64), ProgressLevel::EliteTier);

        // Now age the ledger far beyond the default Soroban persistent TTL
        // (~4096 ledgers) — 100,000 ledgers, well past the archival threshold.
        //
        // The jump is taken in sub-INSTANCE_TTL_MAX steps rather than one leap
        // because the contract *instance* can only ever be extended by
        // INSTANCE_TTL_MAX (500) ledgers. A single 100,000-ledger jump archives
        // the instance itself, and then no call can land at all
        // (Storage/MissingValue on "contract instance") — which would say
        // nothing about the PlayerLevel entry this test is actually about.
        //
        // Each step performs only *reads*. That is precisely the scenario under
        // test: a player record that is never written again must not decay, and
        // `get_level`'s keep-alive is what prevents that. `get_history_count`
        // bumps the instance TTL so the contract stays invocable.
        let step = (INSTANCE_TTL_MIN - 1) as u64;
        let target = 100u64 + 100_000;
        let mut seq = 100u64;
        while seq < target {
            seq = (seq + step).min(target);
            env.ledger().with_mut(|l| {
                l.sequence_number = seq as u32;
            });
            client.get_history_count(&1u64);
            client.get_level(&1u64);
        }

        // CRITICAL: With the fix in place, reads extend the TTL, so the record
        // is still live and returns EliteTier (not Unverified).
        // Without the fix, this either panics (key is archived) or returns Unverified.
        let level_after_dormancy = client.get_level(&1u64);
        assert_eq!(
            level_after_dormancy,
            ProgressLevel::EliteTier,
            "Player level must not silently revert to Unverified after extended dormancy"
        );

        // Verify that subsequent reads also work (keep-alive is continuous).
        assert_eq!(client.get_level(&1u64), ProgressLevel::EliteTier);
    }
}
