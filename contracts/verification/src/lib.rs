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
#![cfg_attr(target_family = "wasm", no_std)]
mod errors;
pub mod events;
mod types;

use errors::VerificationError;
use types::{
    ContractHealth, DataKey, GlobalMilestoneEntry, GlobalMilestoneIndexPage, Milestone,
    MilestoneDispute, MilestoneRef, Validator, ValidatorStatus,
};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

use scoutchain_shared_types::{require_admin, validate_cid};

const MAX_CREDENTIALS_LEN: u32 = 256;
/// Minimum credentials length for validator registration.
/// Credentials must contain at least a short certification identifier
/// (e.g. "UEFA B" = 6 chars) to prevent empty or trivially short strings.
const MIN_CREDENTIALS_LEN: u32 = 10;
const MAX_GLOBAL_MILESTONE_INDEX: u32 = 500;

// Persistent storage TTL bump for milestone records.
const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 2_000;

/// Maximum number of simultaneously registered validators.
/// Increase requires a contract upgrade because the ValidatorVector entry
/// is bounded by Soroban's 64 KB per-entry limit.
const MAX_VALIDATORS: u32 = 100;

/// Maximum milestones a single validator may approve for one player.
const MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR: u32 = 5;

// Core identity TTL: 30 days at ~5s/ledger ≈ 518_400 ledgers.
// Milestone records, validator registrations, and evidence uniqueness data are
// core identity records. A milestone approved by a validator is a permanent
// part of a player's reputation and must not be silently archived.
const PERSISTENT_TTL_MIN: u32 = 500;
const PERSISTENT_TTL_MAX: u32 = 518_400;

// Admin key TTL — synchronized with other contracts to ensure cross-contract
// admin operations remain valid over time.
const ADMIN_BUMP_LEDGERS: u32 = 518_400;

/// Maximum length for milestone description in bytes.
const MAX_DESCRIPTION_LEN: u32 = 256;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Generated client for the progress contract — used for cross-contract calls.
// The progress contract must be deployed and its address registered via
// `set_progress_contract` before `approve_milestone` can advance levels.
mod progress_contract {
    soroban_sdk::contractimport!(file = "fixtures/scoutchain_progress.wasm");
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
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ADMIN_BUMP_LEDGERS,
            ADMIN_BUMP_LEDGERS,
        );
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .set(&DataKey::TotalMilestoneCount, &0u32);
        env.storage()
            .instance()
            .set(&DataKey::ActiveValidatorCount, &0u32);
        env.storage()
            .instance()
            .set(&DataKey::TotalValidatorCount, &0u32);
        env.storage()
            .instance()
            .set(&DataKey::ActiveDisputesCount, &0u32);
        events::contract_initialized(&env, &admin);
        Ok(())
    }

    /// Propose a replacement administrator. The current admin remains active
    /// until the proposed address calls `accept_admin`.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), VerificationError> {
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
    pub fn accept_admin(env: Env) -> Result<(), VerificationError> {
        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;
        let new_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .ok_or(VerificationError::PendingAdminNotSet)?;
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
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), VerificationError> {
        Self::propose_admin(env, new_admin)
    }

    /// Store the progress contract address so approve_milestone can call it.
    /// Must be called after both contracts are deployed (admin only).
    /// Returns AlreadyConfigured if called more than once — use update_progress_contract instead.
    pub fn set_progress_contract(
        env: Env,
        progress_contract: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        if env.storage().instance().has(&DataKey::ProgressContractSet) {
            return Err(VerificationError::AlreadyConfigured);
        }
        env.storage()
            .instance()
            .set(&DataKey::ProgressContract, &progress_contract);
        env.storage()
            .instance()
            .set(&DataKey::ProgressContractSet, &true);
        events::progress_contract_updated(&env, &admin, &progress_contract);
        Ok(())
    }

    /// Update the progress contract address (admin only).
    /// Use this for intentional re-wiring after the initial set_progress_contract call.
    pub fn update_progress_contract(
        env: Env,
        progress_contract: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage()
            .instance()
            .set(&DataKey::ProgressContract, &progress_contract);
        events::progress_contract_updated(&env, &admin, &progress_contract);
        Ok(())
    }

    /// Register a trusted validator (admin only).
    pub fn register_validator(
        env: Env,
        wallet: Address,
        credentials: String,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;

        if credentials.len() > MAX_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        if credentials.len() < MIN_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        // Check if we've reached the maximum number of validators
        let total_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalValidatorCount)
            .unwrap_or(0u32);
        if total_count >= MAX_VALIDATORS {
            return Err(VerificationError::ValidatorCapReached);
        }

        let mut validator_vector: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));

        if env
            .storage()
            .persistent()
            .has(&DataKey::Validator(wallet.clone()))
        {
            return Err(VerificationError::ValidatorAlreadyRegistered);
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
        // Keep-alive: extend TTL for validator records to preserve their identity
        // and active/revoked status over time.
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Validator(wallet.clone()), PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        validator_vector.push_back(wallet.clone());
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validator_vector);
        // Keep-alive: extend TTL for the validator vector itself so the registry
        // remains discoverable.
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::ValidatorVector, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        let active_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ActiveValidatorCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::ActiveValidatorCount,
            &active_count
                .checked_add(1)
                .ok_or(VerificationError::Overflow)?,
        );

        let total_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalValidatorCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::TotalValidatorCount,
            &total_count
                .checked_add(1)
                .ok_or(VerificationError::Overflow)?,
        );

        events::validator_registered(&env, &wallet, &validator.credentials);

        Ok(())
    }
    pub fn get_validators(env: Env) -> Vec<Address> {
        let all: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));
        let mut active = Vec::new(&env);
        for i in 0..all.len() {
            let wallet = all.get(i).unwrap();
            let status = Self::get_validator_status(env.clone(), wallet.clone());
            if status == ValidatorStatus::Active {
                active.push_back(wallet);
            }
        }
        active
    }

    /// Deactivate a validator (admin only).
    /// Optionally accepts a reason (max 128 bytes) that is included in the event.
    pub fn revoke_validator(
        env: Env,
        wallet: Address,
        reason: Option<String>,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

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
        let was_active = validator.active;
        validator.active = false;
        env.storage()
            .persistent()
            .set(&DataKey::Validator(wallet.clone()), &validator);

        if was_active {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ActiveValidatorCount)
                .unwrap_or(0u32);
            env.storage().instance().set(
                &DataKey::ActiveValidatorCount,
                &count.checked_sub(1).ok_or(VerificationError::Overflow)?,
            );
        }

        let validator_vector: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_vector: Vec<Address> = Vec::new(&env);
        for i in 0..validator_vector.len() {
            let addr = validator_vector.get(i).unwrap();
            if addr != wallet {
                new_vector.push_back(addr);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &new_vector);

        let reason_str = reason.unwrap_or(String::from_str(&env, ""));
        events::validator_revoked(&env, &admin, &wallet, &reason_str);

        let routine_str = String::from_str(&env, "Routine");
        if reason_str != routine_str {
            env.storage().persistent().set(&DataKey::ValidatorRevokedForCause(wallet.clone()), &true);
            events::validator_revoked_for_cause(&env, &admin, &wallet, &reason_str);
        }
        Ok(())
    }

    /// Revoke multiple validators in a single atomic transaction (admin only).
    /// Iterates the wallet list and applies the same revoke logic for each,
    /// emitting one `validator_revoked` event per revocation.
    /// If a wallet is not found, the entire batch fails (atomicity).
    pub fn batch_revoke_validators(
        env: Env,
        wallets: Vec<Address>,
        reason: Option<String>,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        if let Some(ref r) = reason {
            if r.len() > 128 {
                return Err(VerificationError::ReasonTooLong);
            }
        }

        let reason_str = reason.unwrap_or(String::from_str(&env, ""));

        for i in 0..wallets.len() {
            let wallet = wallets.get(i).unwrap();

            let mut validator: Validator = env
                .storage()
                .persistent()
                .get(&DataKey::Validator(wallet.clone()))
                .ok_or(VerificationError::ValidatorNotFound)?;
            validator.active = false;
            env.storage()
                .persistent()
                .set(&DataKey::Validator(wallet.clone()), &validator);

            let validator_vector: Vec<Address> = env
                .storage()
                .persistent()
                .get(&DataKey::ValidatorVector)
                .unwrap_or_else(|| Vec::new(&env));
            let mut new_vector: Vec<Address> = Vec::new(&env);
            for j in 0..validator_vector.len() {
                let addr = validator_vector.get(j).unwrap();
                if addr != wallet {
                    new_vector.push_back(addr);
                }
            }
            env.storage()
                .persistent()
                .set(&DataKey::ValidatorVector, &new_vector);

            events::validator_revoked(&env, &admin, &wallet, &reason_str);
            
            let routine_str = String::from_str(&env, "Routine");
            if reason_str != routine_str {
                env.storage().persistent().set(&DataKey::ValidatorRevokedForCause(wallet.clone()), &true);
                events::validator_revoked_for_cause(&env, &admin, &wallet, &reason_str);
            }
        }

        Ok(())
    }
    /// Register multiple validators in a single atomic transaction (admin only).
    ///
    /// Applies the same validation logic as `register_validator` to each entry.
    /// If any entry fails validation (duplicate wallet, credentials length out of bounds,
    /// or the batch would exceed the validator cap), the entire batch fails and no state
    /// changes are persisted.
    pub fn batch_register_validators(
        env: Env,
        entries: Vec<(Address, String)>,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;

        // Preliminary cap check: ensure the batch won't push us over MAX_VALIDATORS.
        let current_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalValidatorCount)
            .unwrap_or(0u32);
        let batch_len = entries.len();
        if current_count
            .checked_add(batch_len as u32)
            .ok_or(VerificationError::Overflow)?
            > MAX_VALIDATORS
        {
            return Err(VerificationError::ValidatorCapReached);
        }

        // First pass: validate each entry without mutating state.
        for i in 0..entries.len() {
            let (wallet, credentials) = entries.get(i).unwrap();

            // Length checks.
            if credentials.len() > MAX_CREDENTIALS_LEN || credentials.len() < MIN_CREDENTIALS_LEN {
                return Err(VerificationError::InvalidInput);
            }

            // Duplicate within the batch.
            for j in 0..i {
                let (other_wallet, _) = entries.get(j).unwrap();
                if other_wallet == wallet {
                    return Err(VerificationError::ValidatorAlreadyRegistered);
                }
            }

            // Duplicate in existing registry.
            if env
                .storage()
                .persistent()
                .has(&DataKey::Validator(wallet.clone()))
            {
                return Err(VerificationError::ValidatorAlreadyRegistered);
            }
        }

        // All validations passed – now persist the new validators.
        let mut validator_vector: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));

        for i in 0..entries.len() {
            let (wallet, credentials) = entries.get(i).unwrap();
            let validator = Validator {
                wallet: wallet.clone(),
                credentials: credentials.clone(),
                registered_at: env.ledger().timestamp(),
                active: true,
            };
            env.storage()
                .persistent()
                .set(&DataKey::Validator(wallet.clone()), &validator);
            // Keep-alive: extend TTL for validator records.
            env.storage()
                .persistent()
                .extend_ttl(&DataKey::Validator(wallet.clone()), PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
            validator_vector.push_back(wallet.clone());

            // Increment active validator count.
            let active_count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ActiveValidatorCount)
                .unwrap_or(0u32);
            env.storage().instance().set(
                &DataKey::ActiveValidatorCount,
                &active_count
                    .checked_add(1)
                    .ok_or(VerificationError::Overflow)?,
            );

            // Increment total validator count.
            let total_count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::TotalValidatorCount)
                .unwrap_or(0u32);
            env.storage().instance().set(
                &DataKey::TotalValidatorCount,
                &total_count
                    .checked_add(1)
                    .ok_or(VerificationError::Overflow)?,
            );

            events::validator_registered(&env, &wallet, &validator.credentials);
        }

        // Persist updated vector.
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validator_vector);
        // Keep-alive: extend TTL for the validator vector.
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::ValidatorVector, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        Ok(())
    }
    /// Re-activate a previously revoked validator (admin only).
    ///
    /// Sets `validator.active = true` so the validator can approve milestones
    /// again immediately without losing their milestone history or credentials
    /// (closes #475).
    ///
    /// Returns `ValidatorNotFound` if the wallet has never been registered.
    pub fn restore_validator(env: Env, wallet: Address) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let mut validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;

        let was_inactive = !validator.active;
        validator.active = true;
        env.storage()
            .persistent()
            .set(&DataKey::Validator(wallet.clone()), &validator);

        if was_inactive {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ActiveValidatorCount)
                .unwrap_or(0u32);
            env.storage().instance().set(
                &DataKey::ActiveValidatorCount,
                &count.checked_add(1).ok_or(VerificationError::Overflow)?,
            );
        }

        env.storage()
            .persistent()
            .remove(&DataKey::ValidatorRevokedForCause(wallet.clone()));

        events::validator_restored(&env, &admin, &wallet);
        Ok(())
    }

    /// Transfer a validator's identity to a new wallet address (admin only).
    ///
    /// Copies the full `Validator` record (with `wallet` updated to `new_wallet`)
    /// to `DataKey::Validator(new_wallet)`, migrates the `ValidatorMilestoneCount`
    /// counter, removes the old storage keys, and replaces `old_wallet` with
    /// `new_wallet` in `ValidatorVector` (closes #476).
    ///
    /// Returns `ValidatorNotFound` if `old_wallet` is not registered.
    /// Returns `ValidatorAlreadyRegistered` if `new_wallet` is already in the registry.
    pub fn transfer_validator(
        env: Env,
        old_wallet: Address,
        new_wallet: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        // Ensure old wallet is registered
        let old_validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(old_wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;

        // Ensure new wallet is not already registered
        if env
            .storage()
            .persistent()
            .has(&DataKey::Validator(new_wallet.clone()))
        {
            return Err(VerificationError::ValidatorAlreadyRegistered);
        }

        // Copy the record with updated wallet field
        let new_validator = Validator {
            wallet: new_wallet.clone(),
            credentials: old_validator.credentials.clone(),
            registered_at: old_validator.registered_at,
            active: old_validator.active,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Validator(new_wallet.clone()), &new_validator);

        // Migrate ValidatorMilestoneCount to new wallet
        let old_count_key = DataKey::ValidatorMilestoneCount(old_wallet.clone());
        let new_count_key = DataKey::ValidatorMilestoneCount(new_wallet.clone());
        let milestone_count: u32 = env
            .storage()
            .persistent()
            .get(&old_count_key)
            .unwrap_or(0u32);
        if milestone_count > 0 {
            env.storage()
                .persistent()
                .set(&new_count_key, &milestone_count);
        }

        // Remove old wallet keys
        env.storage()
            .persistent()
            .remove(&DataKey::Validator(old_wallet.clone()));
        env.storage().persistent().remove(&old_count_key);

        // Replace old_wallet with new_wallet in ValidatorVector
        let mut validator_vector: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env));

        // Find index of old_wallet and replace it
        let mut found_idx: Option<u32> = None;
        for i in 0..validator_vector.len() {
            if validator_vector.get(i).unwrap() == old_wallet {
                found_idx = Some(i);
                break;
            }
        }
        if let Some(idx) = found_idx {
            validator_vector.set(idx, new_wallet.clone());
        }
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validator_vector);

        events::validator_transferred(&env, &admin, &old_wallet, &new_wallet);
        Ok(())
    }

    pub fn pause_contract(env: Env) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    /// Upgrade the contract WASM. Admin auth required.
    /// Persistent storage (including Admin) survives this call.
    pub fn upgrade(
        env: Env,
        new_wasm_hash: soroban_sdk::BytesN<32>,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.deployer().update_current_contract_wasm(new_wasm_hash);
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
    /// NOTE: Age validation of the evidence is the responsibility of the off-chain
    /// validator review process.
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

        if description.len() > MAX_DESCRIPTION_LEN {
            return Err(VerificationError::InvalidInput);
        }

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

        // Global uniqueness check: reject if the evidence has already been used.
        let evidence_used_key = DataKey::EvidenceUsed(evidence_hash.clone());
        if env.storage().persistent().has(&evidence_used_key) {
            return Err(VerificationError::DuplicateEvidence);
        }

        let vp_key = DataKey::ValidatorPlayerMilestoneCount(validator_wallet.clone(), player_id);
        let vp_count: u32 = env.storage().persistent().get(&vp_key).unwrap_or(0u32);
        if vp_count >= MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR {
            return Err(VerificationError::MilestoneLimitExceeded);
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
        // Keep-alive: extend TTL for milestone record to prevent archival of
        // permanently significant reputation events.
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Milestone(player_id, next_index), PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        
        env.storage().persistent().set(&counter_key, &next_index);
        // Keep-alive: extend TTL for the milestone counter so future milestones
        // can be correctly indexed.
        env.storage()
            .persistent()
            .extend_ttl(&counter_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // Mark the evidence hash as globally used
        env.storage().persistent().set(&evidence_used_key, &true);
        // Keep-alive: extend TTL for evidence uniqueness so the same evidence
        // cannot be reused after archival.
        env.storage()
            .persistent()
            .extend_ttl(&evidence_used_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // Increment per-validator milestone count
        let val_key = DataKey::ValidatorMilestoneCount(validator_wallet.clone());
        let val_count: u32 = env.storage().persistent().get(&val_key).unwrap_or(0u32);
        env.storage().persistent().set(
            &val_key,
            &(val_count
                .checked_add(1)
                .ok_or(VerificationError::Overflow)?),
        );
        env.storage()
            .persistent()
            .extend_ttl(&val_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        env.storage().persistent().set(
            &vp_key,
            &(vp_count.checked_add(1).ok_or(VerificationError::Overflow)?),
        );

        // Update ValidatorPlayers index: record that this validator has approved
        // a milestone for player_id. Duplicates are skipped so each player_id
        // appears at most once per validator.
        let vp_index_key = DataKey::ValidatorPlayers(validator_wallet.clone());
        let mut vp_players: Vec<u64> = env
            .storage()
            .persistent()
            .get(&vp_index_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !vp_players.contains(player_id) {
            vp_players.push_back(player_id);
            env.storage().persistent().set(&vp_index_key, &vp_players);
        }

        // Increment global total milestone count
        let total: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalMilestoneCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::TotalMilestoneCount,
            &(total.checked_add(1).ok_or(VerificationError::Overflow)?),
        );

        let mut global_index: Vec<GlobalMilestoneEntry> = env
            .storage()
            .instance()
            .get(&DataKey::GlobalMilestoneIndex)
            .unwrap_or_else(|| Vec::new(&env));
        if global_index.len() >= MAX_GLOBAL_MILESTONE_INDEX {
            global_index.remove(0);
        }
        global_index.push_back(GlobalMilestoneEntry {
            player_id,
            milestone_index: next_index,
        });
        env.storage()
            .instance()
            .set(&DataKey::GlobalMilestoneIndex, &global_index);

        // Record the approval in the validator's compact milestone index.
        // This index is exposed through the validator milestone query methods.
        let validator_milestones_key = DataKey::ValidatorMilestones(validator_wallet.clone());
        let mut validator_milestones: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&validator_milestones_key)
            .unwrap_or_else(|| Vec::new(&env));
        validator_milestones.push_back(MilestoneRef {
            player_id,
            milestone_index: next_index,
        });
        env.storage()
            .persistent()
            .set(&validator_milestones_key, &validator_milestones);

        events::milestone_approved(
            &env,
            player_id,
            &validator_wallet,
            next_index,
            &description,
            &evidence_hash,
        );

        // Cross-contract call: advance the player's progress level.
        // If the progress contract is not wired (e.g. during testing without a
        // full deployment) we emit a diagnostic event and skip advancement so
        // the off-chain indexer can detect the missing wiring.  In production,
        // always call set_progress_contract before going live.
        if let Some(progress_addr) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::ProgressContract)
        {
            let progress_client = progress_contract::Client::new(&env, &progress_addr);
            match progress_client.try_advance_level(&validator_wallet, &player_id, &next_index) {
                Ok(_) => {}
                // AlreadyAtMaxLevel is acceptable — milestone is still recorded.
                // Emit a diagnostic event so the indexer can observe the skip.
                Err(Ok(progress_contract::ProgressError::AlreadyAtMaxLevel)) => {
                    events::level_advancement_skipped(
                        &env,
                        player_id,
                        &soroban_sdk::String::from_str(&env, "AlreadyAtMaxLevel"),
                    );
                }
                // Any other error: emit a diagnostic event then abort.
                // The event appears in the diagnostic stream only (the
                // transaction is reverted), but indexers scanning receipts
                // can use it to alert without parsing raw error codes.
                Err(e) => {
                    let code = match &e {
                        Ok(pe) => *pe as u32,
                        Err(_) => 0u32,
                    };
                    events::progress_call_failed(&env, player_id, code);
                    return Err(VerificationError::ProgressCallFailed);
                }
            }
        } else {
            // Progress contract not configured — emit diagnostic so the indexer
            // can alert on missing wiring rather than silently swallowing it.
            events::progress_contract_not_set(&env, player_id);
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
        let milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, index))
            .ok_or(VerificationError::MilestoneNotFound)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Milestone(player_id, index),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(milestone)
    }

    pub fn get_milestone_with_status(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<types::MilestoneWithValidatorStatus, VerificationError> {
        let milestone = Self::get_milestone(env.clone(), player_id, index)?;
        let validator_status = Self::get_validator_status(env, milestone.validator.clone());
        Ok(types::MilestoneWithValidatorStatus {
            milestone,
            validator_status,
        })
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

    /// Return all distinct player IDs for which the given validator has approved
    /// at least one milestone. The list is accumulated on every `approve_milestone`
    /// call and each player_id appears at most once.
    pub fn get_validator_players(env: Env, wallet: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorPlayers(wallet))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns the number of currently active (non-revoked) validators.
    pub fn get_active_validator_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ActiveValidatorCount)
            .unwrap_or(0u32)
    }

    /// Returns the total number of registered validators (both active and revoked).
    /// Useful as a pre-check before calling register_validator to anticipate
    /// a possible ValidatorCapReached error, since the validator registry is capped
    /// at MAX_VALIDATORS (100).
    pub fn get_validator_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::TotalValidatorCount)
            .unwrap_or(0u32)
    }

    pub fn get_total_milestone_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::TotalMilestoneCount)
            .unwrap_or(0u32)
    }

    pub fn get_active_disputes_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ActiveDisputesCount)
            .unwrap_or(0u32)
    }

    pub fn get_global_milestone_index(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> GlobalMilestoneIndexPage {
        let all: Vec<GlobalMilestoneEntry> = env
            .storage()
            .instance()
            .get(&DataKey::GlobalMilestoneIndex)
            .unwrap_or_else(|| Vec::new(&env));
        let total = all.len();
        let mut entries = Vec::new(&env);
        let cap = if limit > 50 { 50 } else { limit };
        let mut i = offset;
        while i < total && entries.len() < cap {
            entries.push_back(all.get(i).unwrap());
            i += 1;
        }
        GlobalMilestoneIndexPage { entries, total }
    }

    pub fn get_validator(env: Env, wallet: Address) -> Result<Validator, VerificationError> {
        let validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        // Keep-alive: extend TTL on read to preserve validator registration status over time.
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Validator(wallet), PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        Ok(validator)
    }

    /// Return every milestone approved by `wallet`.
    ///
    /// This legacy method is unbounded. High-volume callers should use
    /// `get_validator_milestones_page` to keep response sizes bounded.
    pub fn get_validator_milestones(env: Env, wallet: Address) -> Vec<MilestoneRef> {
        let key = DataKey::ValidatorMilestones(wallet);
        let list: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        if !list.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }
        list
    }

    /// Return a bounded page of milestones approved by `wallet`.
    ///
    /// `limit` is capped at 50 entries, matching `get_global_milestone_index`.
    pub fn get_validator_milestones_page(
        env: Env,
        wallet: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<MilestoneRef> {
        let key = DataKey::ValidatorMilestones(wallet);
        let list: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        if !list.is_empty() {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        let mut page = Vec::new(&env);
        let cap = if limit > 50 { 50 } else { limit };
        let mut i = offset;
        while i < list.len() && page.len() < cap {
            page.push_back(list.get(i).unwrap());
            i += 1;
        }
        page
    }

    /// Return full milestone records for a validator across all players, page by page.
    pub fn get_milestones_by_validator_page(
        env: Env,
        wallet: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<Milestone> {
        let refs: Vec<MilestoneRef> = Self::get_validator_milestones_page(env.clone(), wallet, offset, limit);
        let mut milestones = Vec::new(&env);
        for i in 0..refs.len() {
            let ref_entry = refs.get(i).unwrap();
            if let Ok(milestone) = Self::get_milestone(env.clone(), ref_entry.player_id, ref_entry.milestone_index) {
                milestones.push_back(milestone);
            }
        }
        milestones
    }

    /// Returns the detailed status of a validator wallet.
    pub fn get_validator_status(env: Env, wallet: Address) -> ValidatorStatus {
        let wallet_key = wallet.clone();
        match env
            .storage()
            .persistent()
            .get::<DataKey, Validator>(&DataKey::Validator(wallet_key.clone()))
        {
            None => ValidatorStatus::NotRegistered,
            Some(v) if v.active => ValidatorStatus::Active,
            Some(_) => {
                if env
                    .storage()
                    .persistent()
                    .has(&DataKey::ValidatorRevokedForCause(wallet_key))
                {
                    ValidatorStatus::RevokedForCause
                } else {
                    ValidatorStatus::Revoked
                }
            }
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
    // Milestone dispute (issue #471)
    // -------------------------------------------------------------------------

    /// Allow a player to dispute a milestone they believe was wrongly attributed.
    /// Only the player associated with `player_id` can submit a dispute.
    /// Stores the dispute with reason and timestamp, and emits a `milestone_disputed` event.
    /// Admin can later query disputes and resolve them.
    pub fn dispute_milestone(
        env: Env,
        player_wallet: Address,
        player_id: u64,
        milestone_index: u32,
        reason: String,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        player_wallet.require_auth();

        // Verify the milestone exists
        let milestone: Milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, milestone_index))
            .ok_or(VerificationError::MilestoneNotFound)?;

        // Verify the caller is the player associated with this milestone
        if milestone.player_id != player_id {
            return Err(VerificationError::Unauthorized);
        }

        // Check if dispute already exists
        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);
        if env.storage().persistent().has(&dispute_key) {
            return Err(VerificationError::InvalidInput);
        }

        let dispute = MilestoneDispute {
            player_id,
            milestone_index,
            reason: reason.clone(),
            disputed_at: env.ledger().timestamp(),
            resolved: false,
            upheld: false,
        };

        env.storage().persistent().set(&dispute_key, &dispute);

        let player_disputes_key = DataKey::PlayerDisputes(player_id);
        let mut player_disputes: Vec<u32> = env
            .storage()
            .persistent()
            .get(&player_disputes_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !player_disputes.contains(milestone_index) {
            player_disputes.push_back(milestone_index);
            env.storage()
                .persistent()
                .set(&player_disputes_key, &player_disputes);
            env.storage().persistent().extend_ttl(
                &player_disputes_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ActiveDisputesCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::ActiveDisputesCount,
            &count.checked_add(1).ok_or(VerificationError::Overflow)?,
        );

        events::milestone_disputed(&env, &player_wallet, player_id, milestone_index, &reason);
        Ok(())
    }

    /// Resolve a filed milestone dispute (admin only).
    ///
    /// This marks the dispute as resolved and records whether the admin upheld
    /// it. It does not roll back player progress; that corrective workflow is
    /// intentionally handled separately.
    pub fn resolve_dispute(
        env: Env,
        player_id: u64,
        milestone_index: u32,
        upheld: bool,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);
        let mut dispute: MilestoneDispute = env
            .storage()
            .persistent()
            .get(&dispute_key)
            .ok_or(VerificationError::MilestoneNotFound)?;

        if dispute.resolved {
            return Err(VerificationError::DisputeAlreadyResolved);
        }

        dispute.resolved = true;
        dispute.upheld = upheld;
        env.storage().persistent().set(&dispute_key, &dispute);

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ActiveDisputesCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::ActiveDisputesCount,
            &count.checked_sub(1).ok_or(VerificationError::Overflow)?,
        );

        events::dispute_resolved(&env, &admin, player_id, milestone_index, upheld);
        Ok(())
    }

    /// Query a milestone dispute by player_id and milestone_index.
    pub fn get_dispute(
        env: Env,
        player_id: u64,
        milestone_index: u32,
    ) -> Result<MilestoneDispute, VerificationError> {
        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);
        env.storage()
            .persistent()
            .get(&dispute_key)
            .ok_or(VerificationError::MilestoneNotFound)
    }

    /// Boolean convenience check. Returns `true` if a dispute exists for the
    /// given `(player_id, milestone_index)` pair, `false` otherwise.
    ///
    /// This is a thin read-only wrapper around `get_dispute` — no new storage
    /// is introduced. Mirrors the `is_active_validator` pattern: callers that
    /// only need a yes/no answer avoid handling a `Result`/error path.
    pub fn has_dispute(env: Env, player_id: u64, milestone_index: u32) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::MilestoneDispute(player_id, milestone_index))
    }

    /// Returns the total number of disputes filed for a given `player_id`.
    pub fn get_player_dispute_count(env: Env, player_id: u64) -> u32 {
        let disputes_key = DataKey::PlayerDisputes(player_id);
        if let Some(stored) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<u32>>(&disputes_key)
        {
            stored.len()
        } else {
            let count = Self::get_milestone_count(env.clone(), player_id);
            let mut dispute_count = 0u32;
            for i in 1..=count {
                if env
                    .storage()
                    .persistent()
                    .has(&DataKey::MilestoneDispute(player_id, i))
                {
                    dispute_count += 1;
                }
            }
            dispute_count
        }
    }

    /// Return a paginated list of disputes filed for a given `player_id`.
    ///
    /// `limit` is capped at 50 entries, consistent with pagination elsewhere.
    pub fn get_player_disputes(
        env: Env,
        player_id: u64,
        offset: u32,
        limit: u32,
    ) -> Vec<MilestoneDispute> {
        Self::list_player_disputes_helper(&env, player_id, None, offset, limit)
    }

    /// Return a paginated list of disputes for a player, filtered by resolution status.
    ///
    /// If `resolved` is true, only resolved disputes are returned.
    /// If `resolved` is false, only open/unresolved disputes are returned.
    /// `limit` is capped at 50 entries.
    pub fn get_player_disputes_by_status(
        env: Env,
        player_id: u64,
        resolved: bool,
        offset: u32,
        limit: u32,
    ) -> Vec<MilestoneDispute> {
        Self::list_player_disputes_helper(&env, player_id, Some(resolved), offset, limit)
    }

    fn list_player_disputes_helper(
        env: &Env,
        player_id: u64,
        status_filter: Option<bool>,
        offset: u32,
        limit: u32,
    ) -> Vec<MilestoneDispute> {
        let cap = limit.min(50);
        let mut results = Vec::new(env);
        if cap == 0 {
            return results;
        }

        let disputes_key = DataKey::PlayerDisputes(player_id);
        let indices: Vec<u32> = if let Some(stored) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<u32>>(&disputes_key)
        {
            stored
        } else {
            let count = Self::get_milestone_count(env.clone(), player_id);
            let mut list = Vec::new(env);
            for i in 1..=count {
                if env
                    .storage()
                    .persistent()
                    .has(&DataKey::MilestoneDispute(player_id, i))
                {
                    list.push_back(i);
                }
            }
            list
        };

        let mut skipped = 0u32;
        for i in 0..indices.len() {
            let m_idx = indices.get(i).unwrap();
            if let Ok(dispute) = Self::get_dispute(env.clone(), player_id, m_idx) {
                if let Some(req_resolved) = status_filter {
                    if dispute.resolved != req_resolved {
                        continue;
                    }
                }
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                results.push_back(dispute);
                if results.len() >= cap {
                    break;
                }
            }
        }
        results
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    #[inline(always)]
    fn bump_instance_ttl(env: &Env) {
        const INSTANCE_TTL_MIN: u32 = 100;
        const INSTANCE_TTL_MAX: u32 = 10000;
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_MIN, INSTANCE_TTL_MAX);
    }

    fn require_initialized(env: &Env) -> Result<(), VerificationError> {
        if !env.storage().instance().has(&DataKey::Initialized) {
            return Err(VerificationError::NotInitialized);
        }
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
        testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
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

    // A valid 46-character CIDv0 for use in tests.
    const VALID_CID_V0: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
    // A second, distinct valid CIDv0 — evidence hashes must be globally unique,
    // so tests approving multiple milestones need more than one valid CID.
    const VALID_CID_V0_2: &str = "QmvwxyzABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdef";
    // A third, distinct valid CIDv0.
    const VALID_CID_V0_3: &str = "QmABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdefghijk";
    // A valid CIDv1 (>= 59 chars starting with "bafy").
    const VALID_CID_V1: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

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
            soroban_sdk::vec![
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

        client.pause_contract();
        client.unpause_contract();

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
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
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
                args: soroban_sdk::vec![&env],
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
                args: soroban_sdk::vec![&env],
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
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }]);
        client.accept_admin();
    }

    // -------------------------------------------------------------------------
    // Issue #659: Validator milestone pagination tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_validator_milestones_page_reconstructs_full_history() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Academy Director"));

        // Use distinct players and evidence CIDs so the history exceeds the
        // 50-entry page cap through the normal approval path.
        for player_id in 1u64..=51 {
            let evidence = format!("bafy{:055}", player_id);
            client.approve_milestone(
                &validator,
                &player_id,
                &String::from_str(&env, "approved"),
                &String::from_str(&env, &evidence),
            );
        }

        let full_history = client.get_validator_milestones(&validator);
        assert_eq!(full_history.len(), 51);

        let first_page = client.get_validator_milestones_page(&validator, &0, &50);
        let second_page = client.get_validator_milestones_page(&validator, &50, &50);
        let capped_page = client.get_validator_milestones_page(&validator, &0, &51);
        assert_eq!(first_page.len(), 50);
        assert_eq!(second_page.len(), 1);
        assert_eq!(capped_page.len(), 50);
        assert_eq!(
            client
                .get_validator_milestones_page(&validator, &51, &50)
                .len(),
            0
        );

        let mut reconstructed = Vec::new(&env);
        for page in [first_page, second_page] {
            for i in 0..page.len() {
                reconstructed.push_back(page.get(i).unwrap());
            }
        }
        assert_eq!(reconstructed.len(), full_history.len());
        for i in 0..full_history.len() {
            let expected = full_history.get(i).unwrap();
            let actual = reconstructed.get(i).unwrap();
            assert_eq!(actual.player_id, expected.player_id);
            assert_eq!(actual.milestone_index, expected.milestone_index);
        }
    }

    #[test]
    fn test_get_milestones_by_validator_page_returns_full_records() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Academy Director"));

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "approved"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "second"),
            &String::from_str(&env, VALID_CID_V0_2),
        );

        let page = client.get_milestones_by_validator_page(&validator, &0, &5);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().player_id, 1u64);
        assert_eq!(page.get(0).unwrap().description, String::from_str(&env, "approved"));
        assert_eq!(page.get(1).unwrap().player_id, 2u64);
        assert_eq!(page.get(1).unwrap().evidence_hash, String::from_str(&env, VALID_CID_V0_2));
    }

    // -------------------------------------------------------------------------
    // Issue #466: ValidatorPlayers index tests
    // -------------------------------------------------------------------------

    /// ValidatorPlayers(wallet) index is updated on every approve_milestone call.
    /// get_validator_players returns all player IDs for the given validator.
    /// Duplicate player IDs are not added to the index.
    #[test]
    fn test_get_validator_players_index_accuracy() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Senior Coach"));

        // Unknown validator returns empty vec
        let unknown = Address::generate(&env);
        assert_eq!(client.get_validator_players(&unknown).len(), 0);

        // Approve milestones for players 1, 2, 3 (evidence hashes must be
        // globally unique).
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V0_2),
        );
        client.approve_milestone(
            &validator,
            &3u64,
            &String::from_str(&env, "m3"),
            &String::from_str(&env, VALID_CID_V0_3),
        );

        let players = client.get_validator_players(&validator);
        assert_eq!(players.len(), 3);
        assert!(players.contains(&1u64));
        assert!(players.contains(&2u64));
        assert!(players.contains(&3u64));
    }

    /// Approving a second milestone for the same player must NOT add a duplicate
    /// player_id to the ValidatorPlayers index.
    #[test]
    fn test_get_validator_players_no_duplicates() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Senior Coach"));

        // Approve two milestones for the same player
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V1),
        );

        // player 1 must appear exactly once
        let players = client.get_validator_players(&validator);
        assert_eq!(players.len(), 1);
        assert!(players.contains(&1u64));
    }

    /// Two validators each approve milestones for different players.
    /// Each validator's index must be independent and accurate.
    #[test]
    fn test_get_validator_players_two_validators_independent() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let v1 = Address::generate(&env);
        let v2 = Address::generate(&env);
        client.register_validator(&v1, &String::from_str(&env, "Pro Coach AA"));
        client.register_validator(&v2, &String::from_str(&env, "Pro Coach BB"));

        client.approve_milestone(
            &v1,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &v1,
            &2u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V0_2),
        );
        client.approve_milestone(
            &v2,
            &3u64,
            &String::from_str(&env, "m3"),
            &String::from_str(&env, VALID_CID_V0_3),
        );

        let v1_players = client.get_validator_players(&v1);
        assert_eq!(v1_players.len(), 2);
        assert!(v1_players.contains(&1u64));
        assert!(v1_players.contains(&2u64));
        assert!(!v1_players.contains(&3u64));

        let v2_players = client.get_validator_players(&v2);
        assert_eq!(v2_players.len(), 1);
        assert!(v2_players.contains(&3u64));
    }

    #[test]
    fn test_validator_milestone_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        // Unknown wallet returns 0
        assert_eq!(
            client.get_validator_milestone_count(&Address::generate(&env)),
            0
        );

        let cids = [
            String::from_str(&env, VALID_CID_V0),
            String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqC"),
            String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqD"),
        ];
        for i in 1u64..=3 {
            client.approve_milestone(
                &validator,
                &i,
                &String::from_str(&env, "milestone"),
                &cids[(i - 1) as usize],
            );
        }

        assert_eq!(client.get_validator_milestone_count(&validator), 3);
    }

    #[test]
    fn test_total_milestone_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Initialized to 0
        assert_eq!(client.get_total_milestone_count(), 0);

        let v1 = Address::generate(&env);
        let v2 = Address::generate(&env);
        client.register_validator(&v1, &String::from_str(&env, "UEFA-B-CoachA"));
        client.register_validator(&v2, &String::from_str(&env, "UEFA-B-CoachB"));

        client.approve_milestone(
            &v1,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        assert_eq!(client.get_total_milestone_count(), 1);

        let v0_2 = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqC");
        let v0_3 = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqD");
        client.approve_milestone(&v1, &2u64, &String::from_str(&env, "m2"), &v0_2);
        client.approve_milestone(&v2, &3u64, &String::from_str(&env, "m3"), &v0_3);
        assert_eq!(client.get_total_milestone_count(), 3);

        // per-validator counts still correct
        assert_eq!(client.get_validator_milestone_count(&v1), 2);
        assert_eq!(client.get_validator_milestone_count(&v2), 1);
    }

    #[test]
    fn test_health_false_before_initialize() {
        let (_env, client) = setup();
        assert!(!client.health().initialized);
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
            &String::from_str(&env, VALID_CID_V0),
        );
        assert_eq!(idx, 1);
        assert_eq!(client.get_milestone_count(&1u64), 1);

        let milestone = client.get_milestone(&1u64, &1);
        assert_eq!(milestone.ledger_sequence, env.ledger().sequence());
    }

    #[test]
    fn test_multiple_milestones_same_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let idx1 = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
        );
        let idx2 = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, VALID_CID_V1),
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        let reason: Option<String> = None;
        client.revoke_validator(&validator, &reason);

        // Should panic — validator is inactive
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, VALID_CID_V0),
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
            &String::from_str(&env, VALID_CID_V0),
        );
    }

    #[test]
    fn test_two_validators_approve_milestones_for_same_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator1 = Address::generate(&env);
        let validator2 = Address::generate(&env);
        client.register_validator(&validator1, &String::from_str(&env, "UEFA-B-CoachA"));
        client.register_validator(&validator2, &String::from_str(&env, "UEFA-B-CoachB"));

        client.approve_milestone(
            &validator1,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &validator2,
            &1u64,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, VALID_CID_V1),
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        client.pause_contract();

        // Should panic — contract is paused
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, VALID_CID_V0),
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_approve_milestone_overflow() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

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
            &String::from_str(&env, VALID_CID_V0),
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
                    (
                        Symbol::new(&env, crate::events::CONTRACT_PAUSED),
                        admin.clone(),
                    )
                        .into_val(&env),
                    ().into_val(&env)
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
                    (
                        Symbol::new(&env, crate::events::CONTRACT_UNPAUSED),
                        admin.clone(),
                    )
                        .into_val(&env),
                    ().into_val(&env)
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
                    (
                        Symbol::new(&env, crate::events::PROGRESS_CONTRACT_UPDATED),
                        admin.clone(),
                    )
                        .into_val(&env),
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
    fn test_upgrade_preserves_admin() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let new_wasm_hash = env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::new(&env));
        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.revoke_validator(&validator, &None);
        assert!(!client.is_active_validator(&validator));
    }

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
                    (
                        Symbol::new(&env, crate::events::CONTRACT_INITIALIZED),
                        admin.clone(),
                    )
                        .into_val(&env),
                    ().into_val(&env)
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
    fn test_register_validator_cap_boundary() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Register exactly MAX_VALIDATORS (100) validators — all must succeed.
        for _ in 0..100 {
            let v = Address::generate(&env);
            client.register_validator(&v, &String::from_str(&env, "Credentials"));
        }

        // The 101st registration must return ValidatorCapReached, not panic.
        let extra = Address::generate(&env);
        let result = client.try_register_validator(&extra, &String::from_str(&env, "Credentials"));
        assert_eq!(result, Err(Ok(VerificationError::ValidatorCapReached)));
    }

    #[test]
    fn test_get_validators_excludes_revoked() {
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
        assert_eq!(validators.len(), 2);
        assert!(validators.contains(&v1));
        assert!(!validators.contains(&v2));
        assert!(validators.contains(&v3));
    }

    #[test]
    fn test_get_active_validator_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        assert_eq!(client.get_active_validator_count(), 0);

        let v1 = Address::generate(&env);
        let v2 = Address::generate(&env);
        let v3 = Address::generate(&env);

        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"));
        assert_eq!(client.get_active_validator_count(), 1);

        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"));
        assert_eq!(client.get_active_validator_count(), 2);

        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"));
        assert_eq!(client.get_active_validator_count(), 3);

        let reason: Option<String> = None;
        client.revoke_validator(&v2, &reason);
        assert_eq!(client.get_active_validator_count(), 2);

        client.revoke_validator(&v3, &reason);
        assert_eq!(client.get_active_validator_count(), 1);

        // Revoking an already-revoked validator should not change the count
        client.revoke_validator(&v3, &reason);
        assert_eq!(client.get_active_validator_count(), 1);
    }

    #[test]
    fn test_get_validator_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Initial state: 0 total validators
        assert_eq!(client.get_validator_count(), 0);
        assert_eq!(client.get_validators().len(), 0);

        let v1 = Address::generate(&env);
        let v2 = Address::generate(&env);
        let v3 = Address::generate(&env);

        // Register 3 validators
        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"));
        assert_eq!(client.get_validator_count(), 1);
        assert_eq!(client.get_validators().len(), 1); // get_validators() returns active only, which matches total

        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"));
        assert_eq!(client.get_validator_count(), 2);
        assert_eq!(client.get_validators().len(), 2);

        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"));
        assert_eq!(client.get_validator_count(), 3);
        assert_eq!(client.get_validators().len(), 3);

        // Revoke some validators - total count should remain 3, active count decreases
        let reason: Option<String> = None;
        client.revoke_validator(&v2, &reason);
        assert_eq!(client.get_validator_count(), 3); // total still 3
        assert_eq!(client.get_active_validator_count(), 2); // active decreased to 2
        assert_eq!(client.get_validators().len(), 2); // get_validators() returns active only

        client.revoke_validator(&v3, &reason);
        assert_eq!(client.get_validator_count(), 3); // total still 3
        assert_eq!(client.get_active_validator_count(), 1); // active decreased to 1
        assert_eq!(client.get_validators().len(), 1); // get_validators() returns active only

        // Revoking an already-revoked validator should not change either count
        client.revoke_validator(&v3, &reason);
        assert_eq!(client.get_validator_count(), 3);
        assert_eq!(client.get_active_validator_count(), 1);
    }

    // -------------------------------------------------------------------------
    // #224: CID validation boundary tests
    // -------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_cidv0_too_short_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        // 45 chars starting with Qm — one short of valid CIDv0
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygpq"),
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_cidv0_too_long_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        // 47 chars starting with Qm — one over valid CIDv0
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqBX"),
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_cidv0_invalid_base58_char_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        // 46 chars but contains '0' which is invalid in base58btc
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "Qm0K1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        );
    }

    #[test]
    fn test_cidv0_exactly_46_chars_accepted() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, VALID_CID_V0),
        );
        assert_eq!(idx, 1);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_cidv1_too_short_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        // 58 chars starting with bafy — one short of valid CIDv1
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(
                &env,
                "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzd",
            ),
        );
    }

    #[test]
    fn test_cidv1_exactly_59_chars_accepted() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, VALID_CID_V1),
        );
        assert_eq!(idx, 1);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_no_prefix_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "zdj7WbTaiJT1fgatdet7Sjxf4PJQgXkGfXPFgq5a2SdxYqYg"),
        );
    }

    // -------------------------------------------------------------------------
    // Bug condition exploration test: TTL expiry without bump (Task 1)
    // -------------------------------------------------------------------------

    /// Bug condition exploration test: proves that `get_milestone` does NOT extend
    /// the persistent TTL of `DataKey::Milestone(player_id, index)`.
    ///
    /// Steps:
    ///   1. Initialize contract and register a validator (admin approves a scout as validator)
    ///   2. Call `approve_milestone` to store `DataKey::Milestone(player_id, 1)`
    ///   3. Advance `env.ledger().sequence_number` past the default Soroban persistent TTL
    ///      threshold (100_000 — far above the ~4096 default persistent TTL)
    ///   4. Call `get_milestone(player_id, 1)` and assert it returns the `Milestone` struct
    ///
    /// EXPECTED OUTCOME on UNFIXED code: TEST FAILS — the milestone key has expired,
    /// so `get_milestone` panics or returns `MilestoneNotFound` instead of the `Milestone`.
    /// This failure confirms the bug: reads never extend the TTL.
    #[test]
    fn test_get_milestone_ttl_expires_without_bump() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_id: u64 = 1u64;
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
        );

        // Advance the ledger sequence far past the default Soroban persistent TTL (~4096).
        // After this point, any persistent key written before the advance (without an
        // explicit extend_ttl) will have expired and become inaccessible.
        env.ledger().with_mut(|l| {
            l.sequence_number = 100_000; // well past the ~4096 default persistent TTL
            l.max_entry_ttl = 100_000;
        });

        // On unfixed code this panics because `DataKey::Milestone(player_id, 1)` has expired.
        // The test asserts a successful return — it WILL FAIL on unfixed code, proving the bug.
        let milestone = client.get_milestone(&player_id, &1u32);
        assert_eq!(milestone.player_id, player_id);
    }

    // -------------------------------------------------------------------------
    // Preservation property tests (Task 2)
    // These tests validate that get_milestone's return value and error semantics
    // are unchanged after the TTL-bump fix.
    // -------------------------------------------------------------------------

    /// Property 2: Preservation — get_milestone return value is unchanged.
    ///
    /// Approves a milestone and asserts that every field returned by `get_milestone`
    /// matches the values supplied to `approve_milestone`.
    ///
    /// **Validates: Requirements 3.1**
    #[test]
    fn test_get_milestone_return_value_preserved() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_id: u64 = 42u64;
        let description = String::from_str(&env, "Speed test passed 30 km/h");
        let evidence_hash = String::from_str(&env, VALID_CID_V0);

        let ledger_seq_at_approval = env.ledger().sequence();

        let idx = client.approve_milestone(&validator, &player_id, &description, &evidence_hash);
        assert_eq!(idx, 1);

        // Retrieve the milestone and verify every field matches what was stored.
        let milestone = client.get_milestone(&player_id, &idx);
        assert_eq!(milestone.player_id, player_id);
        assert_eq!(milestone.validator, validator);
        assert_eq!(milestone.description, description);
        assert_eq!(milestone.evidence_hash, evidence_hash);
        assert_eq!(milestone.ledger_sequence, ledger_seq_at_approval);
    }

    /// Property 2: Preservation — get_milestone returns MilestoneNotFound for non-existent entry.
    ///
    /// Calls `get_milestone` for a `(player_id, index)` pair that was never approved and
    /// asserts it returns `MilestoneNotFound`.
    ///
    /// **Validates: Requirements 3.2**
    #[test]
    fn test_get_milestone_not_found_preserved() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let result = client.try_get_milestone(&999u64, &1u32);
        assert!(result.is_err());
    }

    /// Property 2: Preservation — get_milestone does not alter counters.
    ///
    /// Approves a milestone, records the counter values, calls `get_milestone`, and
    /// asserts that both `get_milestone_count` and `get_validator_milestone_count`
    /// remain unchanged.
    ///
    /// **Validates: Requirements 3.3**
    #[test]
    fn test_get_milestone_does_not_alter_counters() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_id: u64 = 7u64;
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Goal scored"),
            &String::from_str(&env, VALID_CID_V0),
        );

        // Snapshot counters before calling get_milestone.
        let milestone_count_before = client.get_milestone_count(&player_id);
        let validator_count_before = client.get_validator_milestone_count(&validator);

        // Call get_milestone — must not change any counters.
        let _milestone = client.get_milestone(&player_id, &1u32);

        // Assert counters are unchanged.
        assert_eq!(
            client.get_milestone_count(&player_id),
            milestone_count_before
        );
        assert_eq!(
            client.get_validator_milestone_count(&validator),
            validator_count_before
        );
    }

    // -------------------------------------------------------------------------
    // get_active_disputes_count tests (#663)
    // -------------------------------------------------------------------------

    /// Count starts at 0 before any disputes are filed.
    #[test]
    fn test_active_disputes_count_starts_at_zero() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        assert_eq!(client.get_active_disputes_count(), 0);
    }

    /// Count increases by 1 for each new dispute on the same milestone.
    #[test]
    fn test_active_disputes_count_increments_on_dispute() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V0_2),
        );

        assert_eq!(client.get_active_disputes_count(), 0);

        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
        );
        assert_eq!(client.get_active_disputes_count(), 1);

        client.dispute_milestone(
            &player_wallet,
            &2u64,
            &1u32,
            &String::from_str(&env, "Also wrong"),
        );
        assert_eq!(client.get_active_disputes_count(), 2);
    }

    /// Count is not affected by dispute_milestone on the same (player, index) —
    /// the duplicate is rejected before the counter increments.
    #[test]
    fn test_active_disputes_count_not_incremented_on_duplicate_dispute() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );

        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "First dispute"),
        );
        assert_eq!(client.get_active_disputes_count(), 1);

        // Second dispute on the same (player, index) should fail
        let result = client.try_dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Second attempt"),
        );
        assert!(result.is_err());
        // Count must remain 1
        assert_eq!(client.get_active_disputes_count(), 1);
    }

    #[test]
    fn test_resolve_dispute_marks_resolved_and_decrements_active_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
        );
        assert_eq!(client.get_active_disputes_count(), 1);

        client.resolve_dispute(&1u64, &1u32, &true);

        let dispute = client.get_dispute(&1u64, &1u32);
        assert!(dispute.resolved);
        assert!(dispute.upheld);
        assert_eq!(client.get_active_disputes_count(), 0);
    }

    #[test]
    fn test_resolve_dispute_emits_event() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.dispute_milestone(
            &player_wallet,
            &2u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
        );

        client.resolve_dispute(&2u64, &1u32, &false);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, crate::events::DISPUTE_RESOLVED),
                        admin.clone(),
                    )
                        .into_val(&env),
                    (2u64, 1u32, false).into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_dispute_milestone_emits_event_with_reason() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );

        let reason = String::from_str(&env, "Wrong attribution");
        client.dispute_milestone(&player_wallet, &2u64, &1u32, &reason);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, "milestone_disputed"),
                        player_wallet.clone()
                    )
                        .into_val(&env),
                    (2u64, 1u32, reason.clone()).into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_resolve_dispute_missing_returns_milestone_not_found() {
        let (_env, client) = setup();
        let admin = Address::generate(&_env);
        client.initialize(&admin);

        let result = client.try_resolve_dispute(&99u64, &1u32, &false);
        assert_eq!(result, Err(Ok(VerificationError::MilestoneNotFound)));
    }

    #[test]
    fn test_resolve_dispute_already_resolved_returns_error() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);
        client.approve_milestone(
            &validator,
            &3u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.dispute_milestone(
            &player_wallet,
            &3u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
        );
        client.resolve_dispute(&3u64, &1u32, &true);

        let result = client.try_resolve_dispute(&3u64, &1u32, &false);
        assert_eq!(result, Err(Ok(VerificationError::DisputeAlreadyResolved)));
        assert_eq!(client.get_active_disputes_count(), 0);
    }

    // -------------------------------------------------------------------------
    // Duplicate validator registration tests
    // -------------------------------------------------------------------------

    // -------------------------------------------------------------------------
    // has_dispute convenience query tests
    // -------------------------------------------------------------------------

    /// `has_dispute` returns `false` before `dispute_milestone` is called and
    /// `true` after, mirroring the `is_active_validator` boolean-helper pattern.
    #[test]
    fn test_has_dispute_false_before_and_true_after_dispute() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);
        let player_id: u64 = 1u64;
        let milestone_index: u32 = 1u32;

        // Approve a milestone so we have something to dispute
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
        );

        // Before dispute: must return false
        assert!(!client.has_dispute(&player_id, &milestone_index));

        // Submit dispute
        client.dispute_milestone(
            &player_wallet,
            &player_id,
            &milestone_index,
            &String::from_str(&env, "Milestone was not completed"),
        );

        // After dispute: must return true
        assert!(client.has_dispute(&player_id, &milestone_index));
    }

    /// `has_dispute` returns `false` for a `(player_id, milestone_index)` pair
    /// that was never disputed, even when other pairs have disputes.
    #[test]
    fn test_has_dispute_false_for_undisputed_milestone() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"));

        let player_wallet = Address::generate(&env);

        // Approve two milestones for player 1
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Milestone one"),
            &String::from_str(&env, VALID_CID_V0),
        );
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Milestone two"),
            &String::from_str(&env, VALID_CID_V1),
        );

        // Dispute only the first milestone
        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Disputed"),
        );

        // The disputed milestone returns true
        assert!(client.has_dispute(&1u64, &1u32));
        // The undisputed milestone returns false
        assert!(!client.has_dispute(&1u64, &2u32));
        // A completely unknown player/index also returns false
        assert!(!client.has_dispute(&999u64, &1u32));
    }

    /// Test that register_validator fails when called with an already-registered wallet.
    ///
    /// Steps:
    ///   1. Initialize contract and register a validator
    ///   2. Attempt to register the same wallet again
    ///   3. Assert the second registration returns ValidatorAlreadyRegistered error
    ///   4. Verify the validator record in storage is unchanged
    ///   5. Verify the ValidatorVector length remains 1 (no duplicate added)
    ///
    /// **Validates: Duplicate registration check in register_validator**
    #[test]
    fn test_register_validator_already_registered_wallet_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let credentials = String::from_str(&env, "UEFA A License");

        // First registration succeeds
        client.register_validator(&validator, &credentials);
        assert!(client.is_active_validator(&validator));

        // Verify validator is in the vector
        let validators = client.get_validators();
        assert_eq!(validators.len(), 1);
        assert_eq!(validators.get(0).unwrap(), validator);

        // Second registration with the same wallet should fail
        let result = client
            .try_register_validator(&validator, &String::from_str(&env, "Different credentials"));
        assert_eq!(
            result,
            Err(Ok(VerificationError::ValidatorAlreadyRegistered))
        );

        // Verify validator record is unchanged after the second call
        let stored_validator = client.get_validator(&validator);
        assert_eq!(stored_validator.wallet, validator);
        assert_eq!(stored_validator.credentials, credentials);
        assert!(stored_validator.active);

        // Verify ValidatorVector length remains 1 (no duplicate added)
        let validators_after = client.get_validators();
        assert_eq!(validators_after.len(), 1);
    }

    #[test]
    fn test_transfer_validator_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let old_wallet = Address::generate(&env);
        let credentials = String::from_str(&env, "UEFA A License");
        client.register_validator(&old_wallet, &credentials);

        // Record a milestone to verify milestones get migrated
        client.approve_milestone(
            &old_wallet,
            &1u64,
            &String::from_str(&env, "Scored 5 goals"),
            &String::from_str(&env, VALID_CID_V0),
        );
        assert_eq!(client.get_validator_milestone_count(&old_wallet), 1);

        let new_wallet = Address::generate(&env);
        client.transfer_validator(&old_wallet, &new_wallet);

        // Verify old wallet is no longer active
        assert!(!client.is_active_validator(&old_wallet));
        assert!(client.try_get_validator(&old_wallet).is_err());

        // Verify new wallet is active and credentials are correct
        assert!(client.is_active_validator(&new_wallet));
        let stored_validator = client.get_validator(&new_wallet);
        assert_eq!(stored_validator.wallet, new_wallet);
        assert_eq!(stored_validator.credentials, credentials);

        // Verify milestone count migrated
        assert_eq!(client.get_validator_milestone_count(&new_wallet), 1);
        assert_eq!(client.get_validator_milestone_count(&old_wallet), 0);

        // Verify ValidatorVector contains new_wallet and not old_wallet
        let validators = client.get_validators();
        assert_eq!(validators.len(), 1);
        assert_eq!(validators.get(0).unwrap(), new_wallet);
    }

    #[test]
    fn test_transfer_validator_same_address() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet = Address::generate(&env);
        let credentials = String::from_str(&env, "UEFA B License");
        client.register_validator(&wallet, &credentials);

        // Record a milestone to verify milestone count remains intact
        client.approve_milestone(
            &wallet,
            &1u64,
            &String::from_str(&env, "Scored 5 goals"),
            &String::from_str(&env, VALID_CID_V0),
        );
        assert_eq!(client.get_validator_milestone_count(&wallet), 1);

        // Call transfer_validator with identical old_wallet and new_wallet
        // This should return Err(Ok(VerificationError::ValidatorAlreadyRegistered))
        let result = client.try_transfer_validator(&wallet, &wallet);
        assert_eq!(
            result,
            Err(Ok(VerificationError::ValidatorAlreadyRegistered))
        );

        // Verify validator is still active and registered
        assert!(client.is_active_validator(&wallet));
        let stored_validator = client.get_validator(&wallet);
        assert_eq!(stored_validator.wallet, wallet);
        assert_eq!(stored_validator.credentials, credentials);

        // Verify milestone count remains intact
        assert_eq!(client.get_validator_milestone_count(&wallet), 1);

        // Verify ValidatorVector length remains 1 and contains wallet
        let validators = client.get_validators();
        assert_eq!(validators.len(), 1);
        assert_eq!(validators.get(0).unwrap(), wallet);
    }

    #[test]
    fn test_validator_reputation_mechanism() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallet_cause = Address::generate(&env);
        let wallet_routine = Address::generate(&env);
        client.register_validator(&wallet_cause, &String::from_str(&env, "Coach A"));
        client.register_validator(&wallet_routine, &String::from_str(&env, "Coach B"));

        // Approve milestones
        client.approve_milestone(&wallet_cause, &1u64, &String::from_str(&env, "M1"), &String::from_str(&env, "QmCause"));
        client.approve_milestone(&wallet_routine, &2u64, &String::from_str(&env, "M2"), &String::from_str(&env, "QmRoutine"));

        // Revoke with cause
        client.revoke_validator(&wallet_cause, &Some(String::from_str(&env, "Misconduct")));
        
        // Revoke for routine
        client.revoke_validator(&wallet_routine, &Some(String::from_str(&env, "Routine")));

        // Check validator status
        assert_eq!(client.get_validator_status(&wallet_cause), types::ValidatorStatus::RevokedForCause);
        assert_eq!(client.get_validator_status(&wallet_routine), types::ValidatorStatus::Revoked);

        // Check milestone with status
        let milestone_cause = client.get_milestone_with_status(&1u64, &1u32);
        assert_eq!(milestone_cause.validator_status, types::ValidatorStatus::RevokedForCause);

        let milestone_routine = client.get_milestone_with_status(&2u64, &1u32);
        assert_eq!(milestone_routine.validator_status, types::ValidatorStatus::Revoked);
        
        // Restore validator and verify the flag is cleared
        client.restore_validator(&wallet_cause);
        assert_eq!(client.get_validator_status(&wallet_cause), types::ValidatorStatus::Active);
        let milestone_restored = client.get_milestone_with_status(&1u64, &1u32);
        assert_eq!(milestone_restored.validator_status, types::ValidatorStatus::Active);
    }
}
