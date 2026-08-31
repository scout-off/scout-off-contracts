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

pub use errors::VerificationError;
pub use types::{
    AttestationStatus, ContractHealth, DataKey, DiversityConfig, DisputeVote, GlobalMilestoneEntry,
    GlobalMilestoneIndexPage, JuryConfig, Milestone, MilestoneAttestation, MilestoneDispute,
    MilestoneRef, MilestoneRefPage, MilestoneWithValidatorStatus, PendingMilestoneClaim,
    PendingVoteRef, RevocationRecord, RevocationSeverity, Validator, ValidatorActivityReport,
    ValidatorPlayersPage, ValidatorStatus, VerificationWiringState,
};

use soroban_sdk::xdr::ToXdr;
use soroban_sdk::IntoVal;
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Bytes, BytesN, Env, String, Symbol, Val, Vec,
};

use scoutchain_shared_types::{
    read_wiring_link, require_admin,
    safe_math::{safe_add_u32, safe_add_u64, safe_sub_u32},
    validate_cid, write_wiring_link, ProgressLevel,
};

const MAX_CREDENTIALS_LEN: u32 = 256;
/// Minimum credentials length for validator registration.
/// Credentials must contain at least a short certification identifier
/// (e.g. "UEFA B" = 6 chars) to prevent empty or trivially short strings.
const MIN_CREDENTIALS_LEN: u32 = 10;
const MAX_GLOBAL_MILESTONE_INDEX: u32 = 500;

/// Maximum number of simultaneously registered validators.
/// Increase requires a contract upgrade because the ValidatorVector entry
/// is bounded by Soroban's 64 KB per-entry limit.
const MAX_VALIDATORS: u32 = 100;

/// Maximum milestones a single validator may approve for one player.
const MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR: u32 = 5;

/// Maximum number of milestones flagged per call in a for-cause revocation
/// cascade sweep.  Keeps per-call CPU cost proportional to this limit rather
/// than to the validator's total historical approval count.  A validator with
/// more than this many prior approvals requires one or more follow-up calls to
/// `continue_revocation_cascade` to complete the sweep.
///
/// 50 matches the pagination cap used throughout the codebase (e.g.
/// `get_validator_milestones_page`, `expire_trial_offers`).
const CASCADE_LIMIT: u32 = 50;

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

/// Maximum number of specialization tags per validator.
const MAX_SPECIALIZATIONS: u32 = 10;

/// Maximum length of a single specialization tag in bytes.
const MAX_SPECIALIZATION_TAG_LEN: u32 = 64;

const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default per-wallet validator registration cooldown (seconds).
const DEFAULT_REG_COOLDOWN_SECS: u64 = 0;

/// Domain separator for off-chain milestone attestation messages.
const ATTESTATION_DOMAIN: &str = "ScoutChain-MilestoneAttestation-v1";

// ── k-of-n threshold milestone attestation ──
//
// Soroban validators cannot practically co-sign a single transaction (they
// are geographically distributed coaches/academy directors — see README
// "Validator Network" — who transact independently, potentially hours or
// days apart). `attest_milestone` accumulates independent votes in storage
// until `threshold` distinct, currently-active validators have attested to
// the same (player_id, evidence_hash) claim, only then committing the
// milestone and cross-calling `progress.advance_level`.

/// Default k-of-n distinct-validator threshold for `attest_milestone`.
///
/// `1` reproduces today's single-signature trust model, so every existing
/// integrator (registration/scout_access/chaos-tests, and this contract's
/// own pre-existing test suite) that calls `approve_milestone` keeps working
/// unchanged with no coordinated migration. This is a deliberate, well-gated
/// degenerate case — NOT a silent escape hatch: the instant an operator
/// raises the threshold via `set_milestone_threshold`, `approve_milestone`
/// itself starts rejecting calls with `ThresholdModeRequiresAttestation`
/// (see `approve_milestone`), so there is no path to bypass a configured
/// k-of-n policy once one is configured. Operators handling real milestone
/// commitments MUST call `set_milestone_threshold(n)` with `n >= 2` to
/// actually close the single-compromised-validator gap this mechanism
/// exists to close.
const DEFAULT_MILESTONE_THRESHOLD: u32 = 1;

/// Default attestation voting window: 14 days. Long enough for a second,
/// independently-transacting validator to notice and corroborate evidence;
/// short enough that sub-threshold claims do not accumulate as effectively
/// permanent dead storage.
const DEFAULT_VOTING_WINDOW_SECS: u64 = 1_209_600;
/// Floor for admin-configured voting windows (1 hour) — prevents an
/// operator from misconfiguring a window so short that no two independently
/// transacting validators could ever both land inside it.
const MIN_VOTING_WINDOW_SECS: u64 = 3_600;
/// Ceiling for admin-configured voting windows (90 days) — bounds how long a
/// sub-threshold claim's fixed-size storage entry can sit unresolved.
const MAX_VOTING_WINDOW_SECS: u64 = 7_776_000;

/// Hard cap on how many distinct sub-threshold claims a single validator may
/// have an open vote on at once. Bounds `DataKey::ValidatorPendingVotes` to a
/// fixed maximum size so `revoke_validator` can always retroactively
/// invalidate a revoked validator's pending votes in O(cap) instead of an
/// unbounded scan over every claim that has ever existed.
const MAX_PENDING_VOTES_PER_VALIDATOR: u32 = 25;

/// Milestone index at/after which the `min_region_quorum` gate applies.
///
/// The player starts at Level 0 and each committed milestone advances one
/// tier, so milestone #2 is the one that advances a player to Level 2
/// (Performance Milestones) and #3 to Level 3 (Elite Tier) — the two gated
/// levels described in the README Progress Level table and
/// `set_min_region_quorum`. Level 0→1 (identity verification, milestone #1)
/// is deliberately not region-gated.
const REGION_QUORUM_GATED_LEVEL_START_MILESTONE: u32 = 2;

// Generated client for the progress contract — used for cross-contract calls.
// The progress contract must be deployed and its address registered via
// `set_progress_contract` before `approve_milestone` can advance levels.
mod progress_contract {
    soroban_sdk::contractimport!(file = "fixtures/scoutchain_progress.wasm");
}

// Types mirroring the registration contract's `get_player` return value,
// used by `dispute_milestone` for the wallet↔player_id authorization check
// (issue #1014).
#[contracttype]
#[derive(Clone, Debug)]
pub struct RegPlayerVitals {
    pub age: u32,
    pub position: String,
    pub region: String,
    pub nationality: String,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RegPlayerProfile {
    pub player_id: u64,
    pub wallet: Address,
    pub vitals: RegPlayerVitals,
    pub ipfs_hashes: Vec<String>,
    pub level: ProgressLevel,
    pub registered_at: u64,
    pub updated_at: u64,
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

    pub fn get_diversity_config(env: Env) -> Option<DiversityConfig> {
        env.storage().persistent().get(&DataKey::DiversityConfig)
    }

    pub fn set_diversity_config(
        env: Env,
        required_distinct_affiliations: u32,
        starting_milestone_index: u32,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let config = DiversityConfig {
            required_distinct_affiliations,
            starting_milestone_index,
        };
        env.storage()
            .persistent()
            .set(&DataKey::DiversityConfig, &config);
        Ok(())
    }

    pub fn set_progress_contract(
        env: Env,
        progress_contract: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        if env.storage().instance().has(&DataKey::ProgressContractSet) {
            return Err(VerificationError::AlreadyConfigured);
        }
        let epoch = write_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
            &progress_contract,
        );
        env.storage()
            .instance()
            .set(&DataKey::ProgressContractSet, &true);
        events::progress_contract_updated(&env, &admin, &progress_contract);
        events::wiring_updated(&env, &admin, "progress_contract", &progress_contract, epoch);
        Ok(())
    }

    /// Re-wire the progress contract address (admin only).
    /// Use this for intentional re-wiring after the initial set_progress_contract call.
    pub fn update_progress_contract(
        env: Env,
        progress_contract: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
            &progress_contract,
        );
        events::progress_contract_updated(&env, &admin, &progress_contract);
        events::wiring_updated(&env, &admin, "progress_contract", &progress_contract, epoch);
        Ok(())
    }

    /// Return the configured progress contract address, or `None` if the
    /// link has not been configured. Read-only and requires no auth.
    pub fn get_progress_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::ProgressContract)
    }

    /// Store the registration contract address so `dispute_milestone` can
    /// verify wallet↔player_id binding via a cross-contract call (admin only).
    /// Returns AlreadyConfigured if called more than once — use
    /// `update_registration_contract` instead.
    pub fn set_registration_contract(
        env: Env,
        reg_contract: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        if env
            .storage()
            .instance()
            .has(&DataKey::RegistrationContractSet)
        {
            return Err(VerificationError::AlreadyConfigured);
        }
        let epoch = write_wiring_link(
            &env,
            &DataKey::RegistrationContract,
            &DataKey::RegistrationContractEpoch,
            &reg_contract,
        );
        env.storage()
            .instance()
            .set(&DataKey::RegistrationContractSet, &true);
        events::wiring_updated(&env, &admin, "registration_contract", &reg_contract, epoch);
        Ok(())
    }

    /// Re-wire the registration contract address (admin only).
    /// Use this for intentional re-wiring after the initial `set_registration_contract` call.
    pub fn update_registration_contract(
        env: Env,
        reg_contract: Address,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let epoch = write_wiring_link(
            &env,
            &DataKey::RegistrationContract,
            &DataKey::RegistrationContractEpoch,
            &reg_contract,
        );
        events::wiring_updated(&env, &admin, "registration_contract", &reg_contract, epoch);
        Ok(())
    }

    /// Returns a snapshot of both cross-contract peer address pointers held
    /// by this contract (progress, registration), each with its address and
    /// re-wiring epoch.
    ///
    /// This is a **read-only** function — it does not require auth, does not
    /// modify state, and is intentionally exempt from the pause/init guards
    /// so it remains callable even on a mis-wired or paused contract, matching
    /// `progress.get_wiring_state()`. See `docs/WIRING_REGISTRY_DESIGN.md`.
    pub fn get_wiring_state(env: Env) -> VerificationWiringState {
        let progress_contract = read_wiring_link(
            &env,
            &DataKey::ProgressContract,
            &DataKey::ProgressContractEpoch,
        );
        let registration_contract = read_wiring_link(
            &env,
            &DataKey::RegistrationContract,
            &DataKey::RegistrationContractEpoch,
        );
        VerificationWiringState {
            progress_contract,
            registration_contract,
        }
    }

    /// Set the minimum number of distinct validator regions required before
    /// `approve_milestone` may call `advance_level` for Level-2
    /// (PerformanceMilestones) and Level-3 (EliteTier) transitions.
    ///
    /// - A value of `0` (default) disables the region-quorum check entirely.
    /// - A value of `2` means milestones from validators in at least 2 distinct
    ///   regions must exist for the player before the level advance is allowed.
    ///
    /// The check applies only to Level-2 and Level-3 advances; Level-0 → 1
    /// (identity verification) is not gated by region diversity.
    pub fn set_min_region_quorum(env: Env, min_regions: u32) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage()
            .instance()
            .set(&DataKey::MinRegionQuorum, &min_regions);
        Ok(())
    }

    /// Return the current minimum-distinct-region quorum for Level-2/3 advances.
    /// Returns `0` if never configured (quorum check disabled).
    pub fn get_min_region_quorum(env: Env) -> u32 {
        env.storage()
            .instance()
            .get::<DataKey, u32>(&DataKey::MinRegionQuorum)
            .unwrap_or(0u32)
    }

    /// Set the k-of-n distinct-active-validator threshold required before a
    /// claim accumulated via `attest_milestone` is committed (admin only).
    ///
    /// Must be in `[1, MAX_VALIDATORS]`. `1` reduces to today's
    /// single-signature model — see `DEFAULT_MILESTONE_THRESHOLD` for why
    /// that default exists and why it is not a silent bypass. Raising this
    /// to `>= 2` is what actually closes the single-compromised-validator
    /// gap this mechanism exists for.
    ///
    /// Already-open claims keep the threshold that was in effect when their
    /// current voting round started (`PendingMilestoneClaim::threshold`) —
    /// changing this value only affects claims that start a fresh round
    /// afterward, so an admin cannot retroactively fast-track or invalidate
    /// an in-flight claim by moving the threshold mid-vote.
    pub fn set_milestone_threshold(env: Env, threshold: u32) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        if threshold == 0 || threshold > MAX_VALIDATORS {
            return Err(VerificationError::InvalidInput);
        }
        env.storage()
            .instance()
            .set(&DataKey::MilestoneApprovalThreshold, &threshold);
        Ok(())
    }

    /// Return the current k-of-n milestone approval threshold (default 1 —
    /// see `DEFAULT_MILESTONE_THRESHOLD`).
    pub fn get_milestone_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get::<DataKey, u32>(&DataKey::MilestoneApprovalThreshold)
            .unwrap_or(DEFAULT_MILESTONE_THRESHOLD)
    }

    /// Set the attestation voting window in seconds (admin only). Must be in
    /// `[MIN_VOTING_WINDOW_SECS, MAX_VOTING_WINDOW_SECS]`. A sub-threshold
    /// claim that does not reach threshold within this window expires — see
    /// `attest_milestone`.
    pub fn set_voting_window_secs(env: Env, window_secs: u64) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        if !(MIN_VOTING_WINDOW_SECS..=MAX_VOTING_WINDOW_SECS).contains(&window_secs) {
            return Err(VerificationError::InvalidInput);
        }
        env.storage()
            .instance()
            .set(&DataKey::AttestationVotingWindowSecs, &window_secs);
        Ok(())
    }

    /// Return the current attestation voting window in seconds (default 14
    /// days — see `DEFAULT_VOTING_WINDOW_SECS`).
    pub fn get_voting_window_secs(env: Env) -> u64 {
        env.storage()
            .instance()
            .get::<DataKey, u64>(&DataKey::AttestationVotingWindowSecs)
            .unwrap_or(DEFAULT_VOTING_WINDOW_SECS)
    }

    /// Register a trusted validator (admin only).
    /// `specializations` is optional; pass an empty Vec for a general-purpose validator
    /// that can approve any untagged (general-category) milestone.
    pub fn register_validator(
        env: Env,
        wallet: Address,
        credentials: String,
        affiliation: String,
        region: String,
        specializations: Vec<String>,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        // Per-wallet cooldown: reject rapid re-registration attempts of the
        // same wallet. Mirrors register_player/register_scout in the
        // registration contract.
        Self::enforce_reg_cooldown(&env, &DataKey::ValidatorRegLastSent(wallet.clone()))?;

        if credentials.len() > MAX_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        if credentials.len() < MIN_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        if affiliation.len() > MAX_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        if region.len() > MAX_CREDENTIALS_LEN {
            return Err(VerificationError::InvalidInput);
        }

        // Validate specializations: cap count and tag length
        if specializations.len() > MAX_SPECIALIZATIONS {
            return Err(VerificationError::InvalidInput);
        }
        for i in 0..specializations.len() {
            let tag = specializations.get(i).unwrap();
            if tag.is_empty() || tag.len() > MAX_SPECIALIZATION_TAG_LEN {
                return Err(VerificationError::InvalidInput);
            }
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
            affiliation,
            region,
            registered_at: env.ledger().timestamp(),
            active: true,
            specializations,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Validator(wallet.clone()), &validator);
        // Keep-alive: extend TTL for validator records to preserve their identity
        // and active/revoked status over time.
        env.storage().persistent().extend_ttl(
            &DataKey::Validator(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        validator_vector.push_back(wallet.clone());
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validator_vector);
        // Keep-alive: extend TTL for the validator vector itself so the registry
        // remains discoverable.
        env.storage().persistent().extend_ttl(
            &DataKey::ValidatorVector,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        let active_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ActiveValidatorCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::ActiveValidatorCount,
            &safe_add_u32(active_count, 1).map_err(|_| VerificationError::Overflow)?,
        );

        let total_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalValidatorCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::TotalValidatorCount,
            &safe_add_u32(total_count, 1).map_err(|_| VerificationError::Overflow)?,
        );

        events::validator_registered(&env, &wallet, &validator.credentials);

        // Record cooldown timestamp for future re-registration attempts.
        let now = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorRegLastSent(wallet.clone()), &now);
        env.storage().persistent().extend_ttl(
            &DataKey::ValidatorRegLastSent(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        Ok(())
    }

    /// Set the per-wallet validator registration cooldown in seconds (admin only).
    /// Pass `0` to disable the cooldown entirely.
    pub fn set_reg_cooldown(env: Env, cooldown_secs: u64) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        env.storage()
            .instance()
            .set(&DataKey::RegCooldownSecs(0), &cooldown_secs);
        Ok(())
    }

    /// Return the current validator registration cooldown in seconds.
    /// Returns `DEFAULT_REG_COOLDOWN_SECS` if no override has been set.
    pub fn get_reg_cooldown(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::RegCooldownSecs(0))
            .unwrap_or(DEFAULT_REG_COOLDOWN_SECS)
    }
    /// Return the active validator registry.
    ///
    /// `ValidatorVector` is maintained as an active-only index by
    /// `register_validator`, `revoke_validator`, `restore_validator`, and
    /// `transfer_validator`. Returning it directly avoids re-reading every
    /// `Validator` record just to discover whether an address is active. That
    /// keeps the read footprint bounded to one ledger entry even at the
    /// platform's 100-validator cap; the per-validator records remain the
    /// source of truth for detailed queries.
    pub fn get_validators(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Deactivate a validator (admin only).
    ///
    /// Accepts an explicit `severity` parameter:
    /// - `RevocationSeverity::Routine` — deactivates the validator, no cascade.
    /// - `RevocationSeverity::ForCause` — deactivates the validator and starts a
    ///   bounded cascade sweep that flags every milestone the validator previously
    ///   approved as `MilestonePendingReReview`.  If the validator has more than
    ///   `CASCADE_LIMIT` (50) prior approvals, the sweep stops after flagging the
    ///   first batch and stores a cursor; call `continue_revocation_cascade` to
    ///   finish.
    ///
    /// Optionally accepts a reason (max 128 bytes) included in the event and
    /// stored in the `RevocationRecord`.
    pub fn revoke_validator(
        env: Env,
        wallet: Address,
        severity: RevocationSeverity,
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
                &safe_sub_u32(count, 1).map_err(|_| VerificationError::Overflow)?,
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

        // Retroactively invalidate this validator's contribution to every
        // still-open (sub-threshold) pending attestation claim.
        let invalidated = Self::invalidate_pending_votes_for_validator(&env, &wallet);
        if invalidated > 0 {
            events::validator_pending_votes_invalidated(&env, &admin, &wallet, invalidated);
        }

        let reason_str = reason.unwrap_or(String::from_str(&env, ""));

        // Persist a RevocationRecord for audit purposes.
        let record = RevocationRecord {
            severity: severity.clone(),
            reason: reason_str.clone(),
            revoked_at: env.ledger().timestamp(),
            admin: admin.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::RevocationRecord(wallet.clone()), &record);
        env.storage().persistent().extend_ttl(
            &DataKey::RevocationRecord(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Emit the appropriate revocation event.
        match severity {
            RevocationSeverity::Routine => {
                events::validator_revoked(&env, &admin, &wallet, &reason_str);
            }
            RevocationSeverity::ForCause => {
                env.storage()
                    .persistent()
                    .set(&DataKey::ValidatorRevokedForCause(wallet.clone()), &true);
                events::validator_revoked(&env, &admin, &wallet, &reason_str);
                events::validator_revoked_for_cause(&env, &admin, &wallet, &reason_str);
                // Start (or complete) the bounded cascade sweep.
                Self::run_cascade_sweep(&env, &wallet, 0)?;
            }
        }

        Ok(())
    }

    /// Continue a for-cause revocation cascade sweep that was interrupted
    /// because the validator had more than `CASCADE_LIMIT` prior approvals.
    ///
    /// Call this repeatedly (admin only) until it stops emitting
    /// `revocation_cascade_continued` events and instead emits
    /// `revocation_cascade_complete`.
    ///
    /// Returns `ValidatorNotFound` if the wallet is not registered, or
    /// `Unauthorized` if the caller is not the admin.  If no cascade is in
    /// progress (cursor absent) this is a no-op (all milestones already
    /// flagged) and emits `revocation_cascade_complete` with the total count.
    pub fn continue_revocation_cascade(env: Env, wallet: Address) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        // Verify the validator exists.
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Validator(wallet.clone()))
        {
            return Err(VerificationError::ValidatorNotFound);
        }

        let cursor: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RevocationCascadeCursor(wallet.clone()))
            .unwrap_or(0);

        Self::run_cascade_sweep(&env, &wallet, cursor)?;
        Ok(())
    }

    /// Internal bounded cascade sweep helper.
    ///
    /// Starting from `start_index` (0-based position in `ValidatorMilestones`),
    /// flags up to `CASCADE_LIMIT` milestones as `MilestonePendingReReview`,
    /// emitting `milestone_flagged_for_rereview` for each.
    ///
    /// If the sweep is completed (fewer remaining milestones than the limit),
    /// removes the cursor and emits `revocation_cascade_complete`.
    ///
    /// If the limit is hit before the list is exhausted, persists the next
    /// cursor and emits `revocation_cascade_continued`.
    fn run_cascade_sweep(
        env: &Env,
        wallet: &Address,
        start_index: u32,
    ) -> Result<(), VerificationError> {
        // A direct persistent key per flag would make a 50-item batch exceed
        // Soroban's transaction write-entry limit once the revocation record,
        // validator index, and cursor are included. Store the flags in compact
        // validator-scoped pages instead: one bounded sweep writes at most two
        // page entries while preserving O(1) lookup by the public getter.
        const FLAG_PAGE_SIZE: u32 = 50;

        let milestones_key = DataKey::ValidatorMilestones(wallet.clone());
        let milestones: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&milestones_key)
            .unwrap_or_else(|| Vec::new(env));

        let total = milestones.len();
        let mut flagged_this_call: u32 = 0;
        let mut i = start_index;
        let count_key = DataKey::MilestonePendingReReviewCount(wallet.clone());
        let mut pending_count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let mut page_index = pending_count / FLAG_PAGE_SIZE;
        let mut page: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&DataKey::MilestonePendingReReviewPage(
                wallet.clone(),
                page_index,
            ))
            .unwrap_or_else(|| Vec::new(env));

        // A repeated initial call is allowed to be idempotent. Load the
        // existing references once rather than probing/writing one key per
        // milestone, which is what previously exhausted transaction limits.
        let mut existing: Vec<MilestoneRef> = Vec::new(env);
        if start_index == 0 && pending_count > 0 {
            let page_count = (pending_count + FLAG_PAGE_SIZE - 1) / FLAG_PAGE_SIZE;
            for p in 0..page_count {
                if let Some(entries) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, Vec<MilestoneRef>>(&DataKey::MilestonePendingReReviewPage(
                        wallet.clone(),
                        p,
                    ))
                {
                    for j in 0..entries.len() {
                        existing.push_back(entries.get(j).unwrap());
                    }
                }
            }
        }

        while i < total && flagged_this_call < CASCADE_LIMIT {
            let m_ref = milestones.get(i).unwrap();
            let mut already_flagged = false;
            for j in 0..existing.len() {
                let stored = existing.get(j).unwrap();
                if stored.player_id == m_ref.player_id
                    && stored.milestone_index == m_ref.milestone_index
                {
                    already_flagged = true;
                    break;
                }
            }

            if !already_flagged {
                if page.len() >= FLAG_PAGE_SIZE {
                    env.storage().persistent().set(
                        &DataKey::MilestonePendingReReviewPage(wallet.clone(), page_index),
                        &page,
                    );
                    env.storage().persistent().extend_ttl(
                        &DataKey::MilestonePendingReReviewPage(wallet.clone(), page_index),
                        PERSISTENT_TTL_MIN,
                        PERSISTENT_TTL_MAX,
                    );
                    page_index = page_index.saturating_add(1);
                    page = Vec::new(env);
                }
                page.push_back(m_ref.clone());
                pending_count =
                    safe_add_u32(pending_count, 1).map_err(|_| VerificationError::Overflow)?;
                events::milestone_flagged_for_rereview(
                    env,
                    wallet,
                    m_ref.player_id,
                    m_ref.milestone_index,
                );
            }
            flagged_this_call =
                safe_add_u32(flagged_this_call, 1).map_err(|_| VerificationError::Overflow)?;
            i = safe_add_u32(i, 1).map_err(|_| VerificationError::Overflow)?;
        }

        if page.len() > 0 {
            env.storage().persistent().set(
                &DataKey::MilestonePendingReReviewPage(wallet.clone(), page_index),
                &page,
            );
            env.storage().persistent().extend_ttl(
                &DataKey::MilestonePendingReReviewPage(wallet.clone(), page_index),
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
            env.storage().persistent().set(&count_key, &pending_count);
            env.storage().persistent().extend_ttl(
                &count_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        let cursor_key = DataKey::RevocationCascadeCursor(wallet.clone());
        if i >= total {
            // Sweep complete — remove the cursor.
            env.storage().persistent().remove(&cursor_key);
            events::revocation_cascade_complete(env, wallet, i);
        } else {
            // More to do — persist cursor and signal continuation.
            env.storage().persistent().set(&cursor_key, &i);
            env.storage().persistent().extend_ttl(
                &cursor_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
            events::revocation_cascade_continued(env, wallet, i, flagged_this_call);
        }

        Ok(())
    }
    /// Revoke multiple validators in a single atomic transaction (admin only).
    /// Iterates the wallet list and applies the same revoke logic for each,
    /// emitting one `validator_revoked` event per revocation.
    /// If a wallet is not found, the entire batch fails (atomicity).
    ///
    /// All wallets in the batch receive the same `severity` and `reason`.
    /// For `RevocationSeverity::ForCause`, each validator's cascade sweep is
    /// started inline; use `continue_revocation_cascade` for any validator
    /// whose prior approval history exceeds `CASCADE_LIMIT`.
    pub fn batch_revoke_validators(
        env: Env,
        wallets: Vec<Address>,
        severity: RevocationSeverity,
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

            let invalidated = Self::invalidate_pending_votes_for_validator(&env, &wallet);
            if invalidated > 0 {
                events::validator_pending_votes_invalidated(&env, &admin, &wallet, invalidated);
            }

            // Persist a RevocationRecord for each wallet.
            let record = RevocationRecord {
                severity: severity.clone(),
                reason: reason_str.clone(),
                revoked_at: env.ledger().timestamp(),
                admin: admin.clone(),
            };
            env.storage()
                .persistent()
                .set(&DataKey::RevocationRecord(wallet.clone()), &record);
            env.storage().persistent().extend_ttl(
                &DataKey::RevocationRecord(wallet.clone()),
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );

            match severity {
                RevocationSeverity::Routine => {
                    events::validator_revoked(&env, &admin, &wallet, &reason_str);
                }
                RevocationSeverity::ForCause => {
                    env.storage()
                        .persistent()
                        .set(&DataKey::ValidatorRevokedForCause(wallet.clone()), &true);
                    events::validator_revoked(&env, &admin, &wallet, &reason_str);
                    events::validator_revoked_for_cause(&env, &admin, &wallet, &reason_str);
                    Self::run_cascade_sweep(&env, &wallet, 0)?;
                }
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
        entries: Vec<(Address, String, String, String, Vec<String>)>,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        // Preliminary cap check: ensure the batch won't push us over MAX_VALIDATORS.
        let current_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalValidatorCount)
            .unwrap_or(0u32);
        let batch_len = entries.len();
        if safe_add_u32(current_count, batch_len).map_err(|_| VerificationError::Overflow)?
            > MAX_VALIDATORS
        {
            return Err(VerificationError::ValidatorCapReached);
        }

        // First pass: validate each entry without mutating state.
        for i in 0..entries.len() {
            let (wallet, credentials, affiliation, specializations) = entries.get(i).unwrap();

            // Per-wallet cooldown, same as register_validator.
            Self::enforce_reg_cooldown(&env, &DataKey::ValidatorRegLastSent(wallet.clone()))?;

            if affiliation.len() > MAX_CREDENTIALS_LEN {
                return Err(VerificationError::InvalidInput);
            }
            if region.len() > MAX_CREDENTIALS_LEN {
                return Err(VerificationError::InvalidInput);
            }

            // Length checks.
            if credentials.len() > MAX_CREDENTIALS_LEN || credentials.len() < MIN_CREDENTIALS_LEN {
                return Err(VerificationError::InvalidInput);
            }

            // Specialization checks.
            if specializations.len() > MAX_SPECIALIZATIONS {
                return Err(VerificationError::InvalidInput);
            }
            for k in 0..specializations.len() {
                let tag = specializations.get(k).unwrap();
                if tag.is_empty() || tag.len() > MAX_SPECIALIZATION_TAG_LEN {
                    return Err(VerificationError::InvalidInput);
                }
            }

            // Duplicate within the batch.
            for j in 0..i {
                let (other_wallet, _, _, _, _) = entries.get(j).unwrap();
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
            let (wallet, credentials, affiliation, region, specializations) = entries.get(i).unwrap();
            if affiliation.len() > MAX_CREDENTIALS_LEN {
                return Err(VerificationError::InvalidInput);
            }
            if region.len() > MAX_CREDENTIALS_LEN {
                return Err(VerificationError::InvalidInput);
            }
            let validator = Validator {
                wallet: wallet.clone(),
                credentials: credentials.clone(),
                affiliation: affiliation.clone(),
                region: region.clone(),
                registered_at: env.ledger().timestamp(),
                active: true,
                specializations: specializations.clone(),
            };
            env.storage()
                .persistent()
                .set(&DataKey::Validator(wallet.clone()), &validator);
            // Keep-alive: extend TTL for validator records.
            env.storage().persistent().extend_ttl(
                &DataKey::Validator(wallet.clone()),
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
            validator_vector.push_back(wallet.clone());

            // Increment active validator count.
            let active_count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ActiveValidatorCount)
                .unwrap_or(0u32);
            env.storage().instance().set(
                &DataKey::ActiveValidatorCount,
                &safe_add_u32(active_count, 1).map_err(|_| VerificationError::Overflow)?,
            );

            // Increment total validator count.
            let total_count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::TotalValidatorCount)
                .unwrap_or(0u32);
            env.storage().instance().set(
                &DataKey::TotalValidatorCount,
                &safe_add_u32(total_count, 1).map_err(|_| VerificationError::Overflow)?,
            );

            events::validator_registered(&env, &wallet, &validator.credentials);
        }

        // Persist updated vector.
        env.storage()
            .persistent()
            .set(&DataKey::ValidatorVector, &validator_vector);
        // Keep-alive: extend TTL for the validator vector.
        env.storage().persistent().extend_ttl(
            &DataKey::ValidatorVector,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
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
                &safe_add_u32(count, 1).map_err(|_| VerificationError::Overflow)?,
            );

            // Re-add the wallet to `ValidatorVector` so registry reads
            // (`get_validators`, `get_active_validators`) see it again —
            // `revoke_validator` removes it from the vector, so a restore
            // must put it back to keep the vector in sync with the count.
            let mut validator_vector: Vec<Address> = env
                .storage()
                .persistent()
                .get(&DataKey::ValidatorVector)
                .unwrap_or_else(|| Vec::new(&env));
            let mut already_present = false;
            for i in 0..validator_vector.len() {
                if validator_vector.get(i).unwrap() == wallet {
                    already_present = true;
                    break;
                }
            }
            if !already_present {
                validator_vector.push_back(wallet.clone());
                env.storage()
                    .persistent()
                    .set(&DataKey::ValidatorVector, &validator_vector);
                env.storage().persistent().extend_ttl(
                    &DataKey::ValidatorVector,
                    PERSISTENT_TTL_MIN,
                    PERSISTENT_TTL_MAX,
                );
            }
        }

        env.storage()
            .persistent()
            .remove(&DataKey::ValidatorRevokedForCause(wallet.clone()));

        events::validator_restored(&env, &admin, &wallet);
        Ok(())
    }

    /// Recover an archived (or expired-but-not-evicted) validator entry by
    /// re-extending its TTL to the core-identity policy value (518,400 ledgers).
    ///
    /// On Soroban protocol 23+, reading an archived entry auto-restores it
    /// within the archival grace period. This entrypoint makes that recovery
    /// explicit and operator-driven, then lifts the entry's TTL back to the
    /// full documented lifetime so it cannot silently age into permanent
    /// eviction. It does NOT change `active`/`banned` flags (use
    /// `restore_validator` for reactivation).
    ///
    /// Admin-only. Returns `ValidatorRecordEvicted` if the entry has already
    /// been fully evicted (key absent) and is unrecoverable.
    pub fn restore_validator_record(env: Env, wallet: Address) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let _validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorRecordEvicted)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Validator(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        events::validator_record_restored(&env, &admin, &wallet);
        Ok(())
    }

    /// Recover an archived (or expired-but-not-evicted) milestone entry by
    /// re-extending its TTL to the core-identity policy value (518,400 ledgers).
    ///
    /// See `restore_validator_record` for the protocol-23 archival-recovery
    /// semantics. Admin-only. Returns `MilestoneRecordEvicted` if the entry has
    /// already been fully evicted (key absent) and is unrecoverable.
    pub fn restore_milestone_record(
        env: Env,
        player_id: u64,
        index: u32,
    ) -> Result<(), VerificationError> {
        let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let _milestone: Milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, index))
            .ok_or(VerificationError::MilestoneRecordEvicted)?;
        env.storage().persistent().extend_ttl(
            &DataKey::Milestone(player_id, index),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        events::milestone_record_restored(&env, &admin, player_id, index);
        Ok(())
    }

    /// Update the specialization tags for an existing validator (admin only).
    ///
    /// Replaces the validator's current `specializations` list with the supplied
    /// one. Pass an empty Vec to make the validator general-purpose (untagged).
    ///
    /// Returns `ValidatorNotFound` if the wallet has never been registered.
    pub fn set_validator_specializations(
        env: Env,
        wallet: Address,
        specializations: Vec<String>,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        // Validate specializations
        if specializations.len() > MAX_SPECIALIZATIONS {
            return Err(VerificationError::InvalidInput);
        }
        for i in 0..specializations.len() {
            let tag = specializations.get(i).unwrap();
            if tag.is_empty() || tag.len() > MAX_SPECIALIZATION_TAG_LEN {
                return Err(VerificationError::InvalidInput);
            }
        }

        let mut validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;

        validator.specializations = specializations;
        env.storage()
            .persistent()
            .set(&DataKey::Validator(wallet.clone()), &validator);
        env.storage().persistent().extend_ttl(
            &DataKey::Validator(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

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
            affiliation: old_validator.affiliation.clone(),
            region: old_validator.region.clone(),
            registered_at: old_validator.registered_at,
            active: old_validator.active,
            specializations: old_validator.specializations.clone(),
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
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;

        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause_contract(env: Env) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;

        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    /// Pause the `approve_milestone` function independently (function-scoped circuit breaker).
    /// The whole-contract pause still takes precedence; this enables granular control
    /// when only validator milestone approval needs to be halted (e.g., validator collusion incident).
    /// All other functions (register_validator, revoke_validator, read queries) remain operational.
    /// Admin only.
    pub fn pause_approve_milestone(env: Env) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;

        env.storage()
            .instance()
            .set(&DataKey::PausedApproveMilestone, &true);
        events::approve_milestone_paused(&env, &admin);
        Ok(())
    }

    /// Unpause the `approve_milestone` function.
    /// Admin only.
    pub fn unpause_approve_milestone(env: Env) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(VerificationError::NotInitialized)?;

        env.storage()
            .instance()
            .set(&DataKey::PausedApproveMilestone, &false);
        events::approve_milestone_unpaused(&env, &admin);
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
        milestone_category: Option<String>,
    ) -> Result<u32, VerificationError> {
        Self::require_not_paused(&env)?;
        Self::require_approve_milestone_not_paused(&env)?;
        validator_wallet.require_auth();

        // Once an operator has opted into k-of-n threshold mode there is no
        // single-signature bypass — see `DEFAULT_MILESTONE_THRESHOLD` and
        // `attest_milestone`.
        if Self::get_milestone_threshold(env.clone()) > 1 {
            return Err(VerificationError::ThresholdModeRequiresAttestation);
        }

        if description.len() > MAX_DESCRIPTION_LEN {
            return Err(VerificationError::InvalidInput);
        }

        // Validate the optional category tag length
        if let Some(ref category) = milestone_category {
            if category.is_empty() || category.len() > MAX_SPECIALIZATION_TAG_LEN {
                return Err(VerificationError::InvalidInput);
            }
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

        // Specialization check: when a milestone category is provided, the validator
        // must have that category in their specializations list.  When category is
        // absent the check is skipped entirely, preserving existing behaviour.
        if let Some(ref category) = milestone_category {
            let mut matched = false;
            for i in 0..validator.specializations.len() {
                if validator.specializations.get(i).unwrap() == *category {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return Err(VerificationError::SpecializationMismatch);
            }
        }

        // Single-signature approval: the attesting set is just this one
        // validator, so the diversity gates keep evaluating against the
        // accumulated per-player sets (passing `None`).
        Self::commit_approved_milestone(
            &env,
            &validator_wallet,
            player_id,
            description,
            evidence_hash,
            None,
        )
    }

    /// Cast one independent, asynchronous vote toward a k-of-n threshold
    /// milestone claim.
    ///
    /// Canonical claim identity is `(player_id, evidence_hash)` — NOT
    /// `description`. Two validators submitting independently-worded
    /// descriptions for the same evidence still corroborate the same claim.
    /// Requiring an exact description match instead would let wording
    /// variance alone fracture legitimate consensus (a validator who
    /// paraphrases "hat-trick in cup final" as "3 goals, regional final"
    /// would silently open a second, disjoint claim rather than
    /// corroborating the first) — a subtler and easier-to-trigger griefing
    /// vector than trusting the immutable evidence artifact, which is what
    /// `evidence_hash` already is. The description recorded on the
    /// committed `Milestone` is therefore locked in by the first vote in
    /// each round and is not overwritten by later voters, so the
    /// threshold-reaching validator cannot rewrite the claim's narrative at
    /// the last moment either.
    ///
    /// Storage is bounded and O(1) per vote regardless of how many distinct
    /// validators eventually attest: each vote touches exactly one
    /// fixed-size `PendingMilestoneClaim` record (a counter, not a growing
    /// list of voters) plus one fixed-size per-(claim, validator) existence
    /// marker used for duplicate-vote rejection. See `cost_budget.rs` /
    /// `docs/CONTRACT_REFERENCE.md` for the measured CPU-instruction
    /// evidence that cost does not grow with vote count.
    ///
    /// Once `threshold` distinct, currently-active validators have voted
    /// for the same claim within the voting window, this call commits the
    /// `Milestone` and cross-calls `progress.advance_level`, exactly like
    /// `approve_milestone` did for a single validator — attribution on the
    /// committed record goes to whichever validator's vote happened to
    /// cross the threshold.
    ///
    /// A vote past the configured voting window starts a fresh round,
    /// discarding all prior votes for this claim (see `PendingMilestoneClaim::round`).
    /// If `revoke_validator` is called against a validator with a still-open
    /// vote on a sub-threshold claim, that vote is retroactively invalidated
    /// (the claim's tally is decremented) — see `revoke_validator`.
    pub fn attest_milestone(
        env: Env,
        validator_wallet: Address,
        player_id: u64,
        description: String,
        evidence_hash: String,
    ) -> Result<AttestationStatus, VerificationError> {
        Self::require_not_paused(&env)?;
        Self::require_approve_milestone_not_paused(&env)?;
        validator_wallet.require_auth();

        if description.len() > MAX_DESCRIPTION_LEN {
            return Err(VerificationError::InvalidInput);
        }
        validate_cid(&evidence_hash).map_err(|_| VerificationError::InvalidInput)?;

        let validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(validator_wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        if !validator.active {
            return Err(VerificationError::ValidatorInactive);
        }

        // Already-committed claims (or evidence reused from any other
        // milestone) are rejected up front — an attestation can never be
        // cast against a claim that already has a Milestone on record.
        if env
            .storage()
            .persistent()
            .has(&DataKey::EvidenceUsed(evidence_hash.clone()))
        {
            return Err(VerificationError::DuplicateEvidence);
        }

        let configured_threshold = Self::get_milestone_threshold(env.clone());
        let window_secs = Self::get_voting_window_secs(env.clone());
        let now = env.ledger().timestamp();

        let claim_key = DataKey::PendingMilestoneClaim(player_id, evidence_hash.clone());
        let mut claim: PendingMilestoneClaim = env
            .storage()
            .persistent()
            .get(&claim_key)
            .unwrap_or(PendingMilestoneClaim {
                player_id,
                evidence_hash: evidence_hash.clone(),
                description: description.clone(),
                vote_count: 0,
                round: 0,
                created_at: now,
                threshold: configured_threshold,
                attester_regions: Vec::new(&env),
                attester_affiliations: Vec::new(&env),
            });

        // Expire a stale sub-threshold round: bump `round` and reset the
        // tally in place. Prior votes become unreachable (their storage key
        // is scoped to the old round) without needing to enumerate or
        // delete them — see `DataKey::PendingMilestoneVote`. The attester
        // diversity tallies are scoped to the round too, so they reset with
        // it (the old round's votes no longer count toward anything).
        if claim.vote_count > 0 && now.saturating_sub(claim.created_at) > window_secs {
            claim.round = claim.round.saturating_add(1);
            claim.vote_count = 0;
            claim.created_at = now;
            claim.description = description.clone();
            claim.threshold = configured_threshold;
            claim.attester_regions = Vec::new(&env);
            claim.attester_affiliations = Vec::new(&env);
            events::attestation_window_expired(&env, player_id, &evidence_hash, claim.round);
        }

        let vote_key = DataKey::PendingMilestoneVote(
            player_id,
            evidence_hash.clone(),
            claim.round,
            validator_wallet.clone(),
        );
        if env.storage().persistent().has(&vote_key) {
            return Err(VerificationError::DuplicateAttestation);
        }

        // Bounded per-validator pending-vote cap. Lazily prune entries that
        // reference a claim which has since committed (removed) or moved to
        // a later round (expired), so a validator's legitimate concurrent
        // capacity is not permanently eaten by claims that already resolved.
        let pending_votes_key = DataKey::ValidatorPendingVotes(validator_wallet.clone());
        let existing_refs: Vec<PendingVoteRef> = env
            .storage()
            .persistent()
            .get(&pending_votes_key)
            .unwrap_or_else(|| Vec::new(&env));
        let mut live_refs: Vec<PendingVoteRef> = Vec::new(&env);
        for i in 0..existing_refs.len() {
            let vref = existing_refs.get(i).unwrap();
            // This validator's own ref for the exact claim being voted on
            // right now must be checked against `claim.round` in memory, not
            // by re-reading storage: when this call is itself the one that
            // just bumped the round on expiry (above), storage still shows
            // the pre-bump round until this transaction's writes land further
            // down. Reading storage here would then treat the just-expired
            // round-N ref as still live, double-booking one slot in this
            // validator's MAX_PENDING_VOTES_PER_VALIDATOR budget once the
            // fresh round-(N+1) ref is pushed below.
            if vref.player_id == player_id && vref.evidence_hash == evidence_hash {
                if vref.round == claim.round {
                    live_refs.push_back(vref);
                }
                continue;
            }
            let other_key =
                DataKey::PendingMilestoneClaim(vref.player_id, vref.evidence_hash.clone());
            if let Some(other_claim) = env
                .storage()
                .persistent()
                .get::<DataKey, PendingMilestoneClaim>(&other_key)
            {
                if other_claim.round == vref.round {
                    live_refs.push_back(vref);
                }
            }
        }
        if live_refs.len() >= MAX_PENDING_VOTES_PER_VALIDATOR {
            return Err(VerificationError::TooManyPendingVotes);
        }

        claim.vote_count =
            safe_add_u32(claim.vote_count, 1).map_err(|_| VerificationError::Overflow)?;

        // Record this attester's region/affiliation in the claim's diversity
        // tallies so the region-quorum and affiliation-diversity gates can be
        // evaluated against the full attesting validator set (not just the
        // threshold-crossing validator) when the claim crosses threshold.
        Self::bump_attester_diversity(&mut claim, &validator);

        env.storage().persistent().set(&vote_key, &now);
        env.storage()
            .persistent()
            .extend_ttl(&vote_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        live_refs.push_back(PendingVoteRef {
            player_id,
            evidence_hash: evidence_hash.clone(),
            round: claim.round,
        });
        env.storage()
            .persistent()
            .set(&pending_votes_key, &live_refs);
        env.storage().persistent().extend_ttl(
            &pending_votes_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::attestation_recorded(
            &env,
            &validator_wallet,
            player_id,
            &evidence_hash,
            claim.vote_count,
            claim.threshold,
        );

        if claim.vote_count >= claim.threshold {
            env.storage().persistent().remove(&claim_key);
            // For a genuine k-of-n threshold (threshold > 1) the milestone is
            // approved by the whole attesting set, so the diversity gates must
            // evaluate over that set. Threshold == 1 is the single-signature
            // trust model and keeps the accumulated per-player evaluation used
            // by `approve_milestone` (passing `None`), so the two paths stay
            // behaviourally identical for a single approving validator.
            let attester_diversity = if claim.threshold > 1 {
                Some(AttesterDiversity {
                    regions: Self::distinct_diversity_values(&claim.attester_regions),
                    affiliations: Self::distinct_diversity_values(&claim.attester_affiliations),
                })
            } else {
                None
            };
            let index = Self::commit_approved_milestone(
                &env,
                &validator_wallet,
                player_id,
                claim.description.clone(),
                evidence_hash.clone(),
                attester_diversity,
            )?;
            Ok(AttestationStatus::Committed(index))
        } else {
            let vote_count = claim.vote_count;
            env.storage().persistent().set(&claim_key, &claim);
            env.storage().persistent().extend_ttl(
                &claim_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
            Ok(AttestationStatus::Pending(vote_count))
        }
    }

    /// Return the current accumulator state for a (player_id, evidence_hash)
    /// claim, if one is open. Returns `None` once the claim commits (its
    /// storage is removed at that point) or before any validator has
    /// attested to it.
    pub fn get_pending_claim(
        env: Env,
        player_id: u64,
        evidence_hash: String,
    ) -> Option<PendingMilestoneClaim> {
        env.storage()
            .persistent()
            .get(&DataKey::PendingMilestoneClaim(player_id, evidence_hash))
    }

    /// Whether `validator_wallet` has an active (not-yet-expired,
    /// not-yet-committed) vote recorded for this claim's current round.
    pub fn has_attested(
        env: Env,
        player_id: u64,
        evidence_hash: String,
        validator_wallet: Address,
    ) -> bool {
        let claim: Option<PendingMilestoneClaim> =
            env.storage()
                .persistent()
                .get(&DataKey::PendingMilestoneClaim(
                    player_id,
                    evidence_hash.clone(),
                ));
        match claim {
            Some(c) => {
                // Round-bumping on expiry is lazy — it only happens inside
                // the next `attest_milestone` call for this claim, so a
                // claim can sit in storage past its window with `c.round`
                // still pointing at the stale round. Without this check,
                // this function would report `true` for a vote that will
                // not count toward any future threshold-cross, contradicting
                // its own "not-yet-expired" contract.
                let window = Self::get_voting_window_secs(env.clone());
                let expired = c.vote_count > 0
                    && env.ledger().timestamp().saturating_sub(c.created_at) > window;
                if expired {
                    return false;
                }
                env.storage()
                    .persistent()
                    .has(&DataKey::PendingMilestoneVote(
                        player_id,
                        evidence_hash,
                        c.round,
                        validator_wallet,
                    ))
            }
            None => false,
        }
    }

    /// Whether the claim's current voting round has exceeded the configured
    /// window without reaching threshold. `true` means the next
    /// `attest_milestone` call for this (player_id, evidence_hash) will
    /// start a fresh round rather than counting toward the existing tally.
    /// Returns `false` when no claim is open (nothing to expire).
    pub fn is_attestation_window_expired(env: Env, player_id: u64, evidence_hash: String) -> bool {
        let claim: Option<PendingMilestoneClaim> = env
            .storage()
            .persistent()
            .get(&DataKey::PendingMilestoneClaim(player_id, evidence_hash));
        match claim {
            Some(c) if c.vote_count > 0 => {
                let window = Self::get_voting_window_secs(env.clone());
                env.ledger().timestamp().saturating_sub(c.created_at) > window
            }
            _ => false,
        }
    }

    /// Register an ed25519 public key used to verify off-chain milestone
    /// attestations for `wallet`.
    ///
    /// The key is stored explicitly (not derived from the Stellar G-address) so
    /// validators can register a dedicated attestation keypair. Callers must be
    /// the wallet itself or the contract admin. Rejects the all-zero key.
    pub fn register_attestation_key(
        env: Env,
        wallet: Address,
        public_key: BytesN<32>,
    ) -> Result<(), VerificationError> {
        Self::require_initialized(&env)?;
        Self::require_not_paused(&env)?;
        // Self-authorized: the validator registers their own attestation key.
        wallet.require_auth();

        let zero = BytesN::<32>::from_array(&env, &[0u8; 32]);
        if public_key == zero {
            return Err(VerificationError::InvalidInput);
        }

        // Wallet must already be a registered validator.
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Validator(wallet.clone()))
        {
            return Err(VerificationError::ValidatorNotFound);
        }

        // If this pubkey was previously bound to another wallet, reject.
        if let Some(existing_owner) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::AttestationKeyOwner(public_key.clone()))
        {
            if existing_owner != wallet {
                return Err(VerificationError::InvalidInput);
            }
        }

        // Clear previous reverse index if rotating keys.
        if let Some(old_key) = env
            .storage()
            .persistent()
            .get::<DataKey, BytesN<32>>(&DataKey::AttestationKey(wallet.clone()))
        {
            if old_key != public_key {
                env.storage()
                    .persistent()
                    .remove(&DataKey::AttestationKeyOwner(old_key));
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::AttestationKey(wallet.clone()), &public_key);
        env.storage().persistent().extend_ttl(
            &DataKey::AttestationKey(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        env.storage()
            .persistent()
            .set(&DataKey::AttestationKeyOwner(public_key.clone()), &wallet);
        env.storage().persistent().extend_ttl(
            &DataKey::AttestationKeyOwner(public_key),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(())
    }

    /// Relayer-submitted milestone approval backed by an off-chain ed25519
    /// attestation (issue #703).
    ///
    /// `relayer` pays fees / authorizes the Soroban transaction but need not be
    /// the validator. Validator identity is taken exclusively from the signed
    /// `attestation.validator_wallet` after `env.crypto().ed25519_verify`
    /// succeeds against that wallet's registered attestation key.
    ///
    /// Commits via the same `commit_approved_milestone` path `approve_milestone`
    /// uses, on the strength of exactly one signature — so it is gated by
    /// k-of-n threshold mode identically: once `get_milestone_threshold() > 1`,
    /// this call rejects with `ThresholdModeRequiresAttestation` and
    /// `attest_milestone` becomes the only path to commit. Without this gate,
    /// this relay path would remain a single-signature bypass of a configured
    /// threshold policy even after `approve_milestone` itself was closed.
    pub fn submit_attested_milestone(
        env: Env,
        relayer: Address,
        attestation: MilestoneAttestation,
        signature: BytesN<64>,
    ) -> Result<u32, VerificationError> {
        Self::require_not_paused(&env)?;
        Self::require_approve_milestone_not_paused(&env)?;
        // Relayer authorizes fee payment only — holds no special privilege.
        relayer.require_auth();

        // Once an operator has opted into k-of-n threshold mode, a single
        // validator's off-chain-signed attestation must not be able to
        // commit a milestone unilaterally either — this path calls the same
        // `commit_approved_milestone` as `approve_milestone` on the strength
        // of exactly one signature, so it needs the identical gate or it
        // becomes an unmonitored bypass of the entire threshold mechanism.
        if Self::get_milestone_threshold(env.clone()) > 1 {
            return Err(VerificationError::ThresholdModeRequiresAttestation);
        }

        // Cross-deployment / cross-network binding (checked before verify so a
        // stolen signature for another instance fails closed without relying on
        // signature mismatch alone).
        if attestation.contract_id != env.current_contract_address() {
            return Err(VerificationError::InvalidAttestation);
        }
        if attestation.network_id != env.ledger().network_id() {
            return Err(VerificationError::InvalidAttestation);
        }

        if attestation.description.len() > MAX_DESCRIPTION_LEN {
            return Err(VerificationError::InvalidInput);
        }
        validate_cid(&attestation.evidence_hash).map_err(|_| VerificationError::InvalidInput)?;

        // Load registered pubkey for the wallet named in the signed payload.
        let public_key: BytesN<32> = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationKey(
                attestation.validator_wallet.clone(),
            ))
            .ok_or(VerificationError::AttestationKeyNotFound)?;

        // Identity must match the reverse index for this pubkey (defeats
        // registering key K under wallet A while embedding wallet B in a
        // separately-crafted payload — the signed wallet must own the key).
        let key_owner: Address = env
            .storage()
            .persistent()
            .get(&DataKey::AttestationKeyOwner(public_key.clone()))
            .ok_or(VerificationError::AttestationKeyNotFound)?;
        if key_owner != attestation.validator_wallet {
            return Err(VerificationError::InvalidAttestation);
        }

        let message = Self::attestation_message(&env, &attestation);
        // Host panics on failure → transaction abort. Pre-checks above ensure
        // binding/key errors return typed VerificationError first.
        env.crypto()
            .ed25519_verify(&public_key, &message, &signature);

        // Validator attribution comes from the verified payload only.
        let validator_wallet = attestation.validator_wallet.clone();

        let validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(validator_wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        if !validator.active {
            return Err(VerificationError::ValidatorInactive);
        }

        // Strictly-increasing per-validator nonce (atomic with commit below).
        let nonce_key = DataKey::AttestationNonce(validator_wallet.clone());
        let last_nonce: u64 = env.storage().persistent().get(&nonce_key).unwrap_or(0u64);
        if attestation.nonce <= last_nonce {
            return Err(VerificationError::InvalidNonce);
        }

        // Single-signature path (gated to threshold == 1 by
        // ThresholdModeRequiresAttestation above) — same accumulated
        // diversity evaluation as `approve_milestone`.
        let index = Self::commit_approved_milestone(
            &env,
            &validator_wallet,
            attestation.player_id,
            attestation.description.clone(),
            attestation.evidence_hash.clone(),
            None,
        )?;

        // Persist nonce only after successful commit so a failed commit does
        // not burn the nonce (tx revert would roll this back on-chain anyway).
        env.storage()
            .persistent()
            .set(&nonce_key, &attestation.nonce);
        env.storage()
            .persistent()
            .extend_ttl(&nonce_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        Ok(index)
    }

    /// Return the last consumed attestation nonce for `wallet` (0 if none).
    pub fn get_attestation_nonce(env: Env, wallet: Address) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::AttestationNonce(wallet))
            .unwrap_or(0u64)
    }

    /// Return the registered attestation public key for `wallet`, if any.
    pub fn get_attestation_key(env: Env, wallet: Address) -> Result<BytesN<32>, VerificationError> {
        env.storage()
            .persistent()
            .get(&DataKey::AttestationKey(wallet))
            .ok_or(VerificationError::AttestationKeyNotFound)
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

    /// Return all milestones for a player with `approved_at >= since_timestamp`.
    ///
    /// Mirrors [`progress::get_history_since`] semantics exactly: iterates the
    /// per-player milestone sequence (indices `1..=count`) and filters in-memory
    /// by `approved_at`, returning entries in approval order (oldest first).
    ///
    /// An indexer that already tracks the timestamp of the last milestone it
    /// processed can pass that timestamp to fetch only new approvals, avoiding
    /// a full re-fetch of the player's entire milestone list on every sync.
    ///
    /// Returns an empty `Vec` when the player has no milestones or when none
    /// satisfy the timestamp predicate.
    pub fn get_milestones_since(env: Env, player_id: u64, since_timestamp: u64) -> Vec<Milestone> {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MilestoneCounter(player_id))
            .unwrap_or(0u32);

        let mut result: Vec<Milestone> = Vec::new(&env);
        for i in 1..=count {
            let key = DataKey::Milestone(player_id, i);
            if let Some(milestone) = env.storage().persistent().get::<DataKey, Milestone>(&key) {
                if milestone.approved_at >= since_timestamp {
                    // Keep-alive: extend TTL on read so accessed milestone
                    // records are not silently archived.
                    env.storage().persistent().extend_ttl(
                        &key,
                        PERSISTENT_TTL_MIN,
                        PERSISTENT_TTL_MAX,
                    );
                    result.push_back(milestone);
                }
            }
        }
        result
    }

    pub fn get_validator_milestone_count(env: Env, wallet: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorMilestoneCount(wallet))
            .unwrap_or(0u32)
    }

    /// Returns the number of currently active (non-revoked) validators.
    pub fn get_active_validator_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ActiveValidatorCount)
            .unwrap_or(0u32)
    }

    /// Returns the total number of registered validators (both active and revoked).
    /// Useful as a pre-check before callingregister_validator to anticipate
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

    /// Return a bounded, paginated page of currently-unresolved
    /// `(player_id, milestone_index)` dispute keys, platform-wide.
    ///
    /// The underlying index (`DataKey::OpenDisputeIndex`) is maintained at
    /// write-time: `dispute_milestone` appends an entry and `resolve_dispute`
    /// removes it, so the index always reflects exactly the set of open
    /// disputes — no full scan is required at query time.
    ///
    /// **Pagination**: `offset` is a zero-based item offset into the index;
    /// `limit` is capped at 50 per page, matching the established pagination
    /// convention used by `get_global_milestone_index` and
    /// `get_validator_milestones_page`.
    ///
    /// **Ordering**: entries are returned in insertion order (oldest first).
    pub fn list_disputes_page(env: Env, offset: u32, limit: u32) -> Vec<(u64, u32)> {
        let open_index: Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&DataKey::OpenDisputeIndex)
            .unwrap_or_else(|| Vec::new(&env));

        let total = open_index.len();
        // Return empty immediately if offset is past the end — prevents a
        // large-offset caller from iterating through the whole index before
        // the while-loop condition fires (DoS-shaped under Soroban limits).
        if offset >= total {
            return Vec::new(&env);
        }
        let cap = limit.max(1).min(50);
        let mut page: Vec<(u64, u32)> = Vec::new(&env);
        let mut i = offset;
        while i < total && page.len() < cap {
            page.push_back(open_index.get(i).unwrap());
            i += 1;
        }
        page
    }

    pub fn get_global_milestone_index(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> GlobalMilestoneIndexPage {
        // ── Ring-buffer read ──────────────────────────────────────────────────
        //
        // State layout:
        //   GlobalMilestoneWriteHead  (instance, u32) — monotonic count of all
        //     writes ever; may exceed MAX_GLOBAL_MILESTONE_INDEX once the buffer
        //     has cycled.
        //   GlobalMilestoneSlot(slot) (persistent, GlobalMilestoneEntry) — one
        //     live entry at slot = write_index % MAX_GLOBAL_MILESTONE_INDEX.
        //
        // Pagination semantics (insertion order, oldest-first):
        //   live_count = min(write_head, CAP)
        //   If write_head < CAP: slots 0..write_head are filled in write order.
        //   If write_head >= CAP: oldest slot = write_head % CAP, wrapping around.
        //
        // offset=0 → oldest surviving entry; offset=live_count-1 → newest.

        let write_head: u32 = env
            .storage()
            .instance()
            .get(&DataKey::GlobalMilestoneWriteHead)
            .unwrap_or(0u32);

        let cap = MAX_GLOBAL_MILESTONE_INDEX;
        let live_count = write_head.min(cap);
        let page_cap = limit.min(50);

        let mut entries = Vec::new(&env);

        if live_count == 0 || offset >= live_count || page_cap == 0 {
            return GlobalMilestoneIndexPage {
                entries,
                total: live_count,
            };
        }

        // When write_head < cap the oldest slot is always 0.
        // When write_head >= cap the oldest slot is write_head % cap (that
        // slot was written earliest among the surviving entries and will be
        // overwritten next).
        let oldest_slot = if write_head < cap { 0u32 } else { write_head % cap };

        let mut i = offset;
        while i < live_count && entries.len() < page_cap {
            let slot = (oldest_slot + i) % cap;
            if let Some(entry) = env
                .storage()
                .persistent()
                .get::<DataKey, GlobalMilestoneEntry>(&DataKey::GlobalMilestoneSlot(slot))
            {
                entries.push_back(entry);
            }
            i += 1;
        }

        GlobalMilestoneIndexPage {
            entries,
            total: live_count,
        }
    }

    /// Return milestones for `player_id` approved at or after `since_timestamp`,
    /// in a bounded page.
    ///
    /// This is the bounded replacement for an unbounded time-range scan:
    /// - `limit` is capped at 50 entries per page (matching the rest of the
    ///   pagination contract).
    /// - `offset` is bounded against the player's milestone count so a large
    ///   offset cannot drive an unbounded iteration loop.
    /// - Returns milestones in approval order (oldest first within the page).
    ///
    /// Callers who want all milestones without a time filter should use
    /// `get_milestone_count` + `get_milestone` directly.
    pub fn get_milestones_since_page(
        env: Env,
        player_id: u64,
        since_timestamp: u64,
        offset: u32,
        limit: u32,
    ) -> Vec<Milestone> {
        let counter_key = DataKey::MilestoneCounter(player_id);
        let count: u32 = env
            .storage()
            .persistent()
            .get(&counter_key)
            .unwrap_or(0u32);

        // Bound offset against the collection length before any iteration.
        if offset >= count || count == 0 {
            return Vec::new(&env);
        }

        let cap = limit.max(1).min(50);
        let mut page: Vec<Milestone> = Vec::new(&env);
        let mut i = offset + 1; // milestone indices are 1-based
        let end = count + 1;    // exclusive upper bound (1-based)

        while i < end && page.len() < cap {
            if let Some(m) = env
                .storage()
                .persistent()
                .get::<DataKey, Milestone>(&DataKey::Milestone(player_id, i))
            {
                if m.approved_at >= since_timestamp {
                    page.push_back(m);
                }
            }
            i += 1;
        }
        page
    }

    pub fn get_validator(env: Env, wallet: Address) -> Result<Validator, VerificationError> {
        let validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        // Keep-alive: extend TTL on read to preserve validator registration status over time.
        env.storage().persistent().extend_ttl(
            &DataKey::Validator(wallet),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        Ok(validator)
    }

    /// Return every milestone approved by `wallet`.
    ///
    /// > **Deprecated**: this legacy method is unbounded. High-volume callers should use
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
    ///
    /// > **Deprecated**: use [`get_validator_milestones_page_v2`] which returns a
    /// [`MilestoneRefPage`] with a `total` field so callers know when to stop paging.
    /// This function is retained for backward compatibility.
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

    /// Return a bounded, paginated page of milestones approved by `wallet`,
    /// together with the total milestone count for the validator.
    ///
    /// This is the canonical successor to both `get_validator_milestones`
    /// (unbounded, deprecated) and `get_validator_milestones_page` (bounded
    /// but no total).  Use this function for new callers — having `total`
    /// lets a client know exactly when paging is complete without over-fetching.
    ///
    /// **Pagination**: `offset` is a zero-based item offset into the validator's
    /// milestone list.  `limit` is capped at 50 entries per page, matching the
    /// convention used by `get_global_milestone_index` and `list_disputes_page`.
    ///
    /// **Ordering**: entries are returned in approval order (oldest first),
    /// exactly as they appear in `ValidatorMilestones` storage.
    pub fn get_validator_milestones_page_v2(
        env: Env,
        wallet: Address,
        offset: u32,
        limit: u32,
    ) -> MilestoneRefPage {
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

        let total = list.len();
        let cap = limit.min(50);
        let mut entries = Vec::new(&env);
        let mut i = offset;
        while i < total && entries.len() < cap {
            entries.push_back(list.get(i).unwrap());
            i += 1;
        }
        MilestoneRefPage { entries, total }
    }

    /// Return all distinct player IDs for which the given validator has approved
    /// at least one milestone. The list is accumulated on every `approve_milestone`
    /// call and each player_id appears at most once.
    ///
    /// > **Deprecated**: this legacy method is unbounded.  High-volume callers
    /// should use [`get_validator_players_page`] to keep response sizes bounded.
    pub fn get_validator_players(env: Env, wallet: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorPlayers(wallet))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Return a bounded, paginated page of distinct player IDs for which
    /// `wallet` has approved at least one milestone, together with the total
    /// player count for the validator.
    ///
    /// This is the canonical paginated successor to the unbounded
    /// `get_validator_players`.  The `total` field lets callers determine when
    /// paging is complete without over-fetching.
    ///
    /// **Pagination**: `offset` is a zero-based item offset; `limit` is capped
    /// at 50 entries per page, matching the convention used by
    /// `get_global_milestone_index` and `get_validator_milestones_page_v2`.
    ///
    /// **Ordering**: entries are returned in the order in which the validator
    /// first approved a milestone for each player (oldest first).
    pub fn get_validator_players_page(
        env: Env,
        wallet: Address,
        offset: u32,
        limit: u32,
    ) -> ValidatorPlayersPage {
        let list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorPlayers(wallet))
            .unwrap_or_else(|| Vec::new(&env));

        let total = list.len();
        let cap = limit.min(50);
        let mut entries = Vec::new(&env);
        let mut i = offset;
        while i < total && entries.len() < cap {
            entries.push_back(list.get(i).unwrap());
            i += 1;
        }
        ValidatorPlayersPage { entries, total }
    }

    /// Return full milestone records for a validator across all players, page by page.
    pub fn get_milestones_by_validator_page(
        env: Env,
        wallet: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<Milestone> {
        let refs: Vec<MilestoneRef> =
            Self::get_validator_milestones_page(env.clone(), wallet, offset, limit);
        let mut milestones = Vec::new(&env);
        for i in 0..refs.len() {
            let ref_entry = refs.get(i).unwrap();
            if let Ok(milestone) =
                Self::get_milestone(env.clone(), ref_entry.player_id, ref_entry.milestone_index)
            {
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

    /// Batch-fetch the status of up to 20 validator wallets in a single call.
    ///
    /// Returns one `ValidatorStatus` entry per input wallet — including
    /// `NotRegistered` for wallets that have never been registered. The
    /// result vector is the same length and in the same order as `wallets`.
    ///
    /// This design is preferred over the silent-skip pattern used by
    /// `registration.get_players`, because `ValidatorStatus` already has a
    /// `NotRegistered` variant that makes the unregistered case
    /// unambiguously representable. Callers always get back exactly N
    /// entries for N inputs, so there is no guessing about which inputs were
    /// "skipped".
    ///
    /// **Batch-size cap**: `wallets` is capped at 20 entries, consistent with
    /// `registration.get_players`. If more than 20 wallets are supplied the
    /// first 20 are processed and the rest are silently ignored — call again
    /// with the remainder if needed.
    pub fn get_validator_statuses(env: Env, wallets: Vec<Address>) -> Vec<ValidatorStatus> {
        const BATCH_CAP: u32 = 20;
        let count = wallets.len().min(BATCH_CAP);
        let mut result = Vec::new(&env);
        for i in 0..count {
            let wallet = wallets.get(i).unwrap();
            result.push_back(Self::get_validator_status(env.clone(), wallet));
        }
        result
    }

    /// Deprecated: use `get_validator_status` instead.
    /// Returns true only for registered, active validators.
    pub fn is_active_validator(env: Env, wallet: Address) -> bool {
        Self::get_validator_status(env, wallet) == ValidatorStatus::Active
    }

    /// Convenience aggregate query — bundles the data from four individual
    /// queries into one call, reducing round-trips for admin dashboards.
    ///
    /// Equivalent to calling:
    /// 1. `get_validator(wallet)`          → credentials, registered_at, active
    /// 2. `get_validator_status(wallet)`   → ValidatorStatus
    /// 3. `get_validator_milestone_count(wallet)` → milestone_count
    /// 4. `get_validator_players(wallet)`  → distinct_players list
    ///
    /// Returns `ValidatorNotFound` if the wallet has never been registered.
    /// This is a pure read-only aggregation — no new storage or business logic.
    pub fn get_validator_activity_report(
        env: Env,
        wallet: Address,
    ) -> Result<ValidatorActivityReport, VerificationError> {
        // 1. Fetch the full Validator record (errors if not registered)
        let validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        // Keep-alive: same as get_validator
        env.storage().persistent().extend_ttl(
            &DataKey::Validator(wallet.clone()),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // 2. Compute status (same logic as get_validator_status)
        let status = Self::get_validator_status(env.clone(), wallet.clone());

        // 3. Milestone count (same logic as get_validator_milestone_count)
        let milestone_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorMilestoneCount(wallet.clone()))
            .unwrap_or(0u32);

        // 4. Distinct players (same logic as get_validator_players)
        let distinct_players: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ValidatorPlayers(wallet.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let distinct_player_count = distinct_players.len();

        Ok(ValidatorActivityReport {
            wallet,
            credentials: validator.credentials,
            registered_at: validator.registered_at,
            active: validator.active,
            status,
            milestone_count,
            distinct_player_count,
            distinct_players,
        })
    }

    // -------------------------------------------------------------------------
    // Migration window management
    // -------------------------------------------------------------------------

    /// Open the migration window.  Admin-only.
    ///
    /// While open, `admin_seed_milestone` and `admin_seed_dispute` may be
    /// called to replay historical records.  Close immediately after replay
    /// with `close_migration_window`.
    pub fn open_migration_window(env: Env) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MigrationActive, &true);
        Ok(())
    }

    /// Close the migration window.  Admin-only.
    pub fn close_migration_window(env: Env) -> Result<(), VerificationError> {
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

    // -------------------------------------------------------------------------
    // Migration seeding
    // -------------------------------------------------------------------------

    /// Seed a historical `Milestone` from a prior contract deployment.
    ///
    /// This is the migration entrypoint for **milestone records**
    /// (MIGRATION_GAPS row 6).  It reconstructs ALL derived state that
    /// `approve_milestone` writes:
    ///
    /// - `Milestone(player_id, milestone_index)` — the record itself
    /// - `EvidenceUsed(evidence_hash)` — global uniqueness index
    /// - `MilestoneCounter(player_id)` — per-player count
    /// - `TotalMilestoneCount` — platform-wide count
    /// - `ValidatorMilestoneCount(validator)` — per-validator count
    /// - `ValidatorPlayerMilestoneCount(validator, player_id)` — per-(validator,player) count
    /// - ring-buffer global audit index (capped at `MAX_GLOBAL_MILESTONE_INDEX`)
    /// - `ValidatorMilestones(validator)` — compact per-validator index
    /// - `ValidatorPlayers(validator)` — per-validator distinct-player index
    ///
    /// ## Idempotency
    ///
    /// Keyed on `(player_id, milestone_index)`.  Byte-identical replay → no-op.
    /// Conflicting content → `MilestoneAlreadyExists`.
    ///
    /// ## EvidenceUsed uniqueness
    ///
    /// If `evidence_hash` is already mapped to a *different*
    /// `(player_id, milestone_index)`, returns `DuplicateEvidence`.
    ///
    /// ## Index ordering
    ///
    /// `milestone_index` is 1-based, matching `commit_approved_milestone`: the
    /// first milestone is index 1 and each subsequent seed must be the next
    /// sequential index (`MilestoneCounter(player_id) + 1`).  Zero, gaps, or
    /// out-of-order indices return `MilestoneNotFound`.
    pub fn admin_seed_milestone(
        env: Env,
        player_id: u64,
        milestone_index: u32,
        milestone: Milestone,
        validator: Address,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        let ms_key = DataKey::Milestone(player_id, milestone_index);

        // ── Idempotency ───────────────────────────────────────────────────────
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, Milestone>(&ms_key)
        {
            let identical = existing.player_id == milestone.player_id
                && existing.validator == milestone.validator
                && existing.description == milestone.description
                && existing.evidence_hash == milestone.evidence_hash
                && existing.approved_at == milestone.approved_at
                && existing.ledger_sequence == milestone.ledger_sequence;
            if identical {
                return Ok(());
            }
            return Err(VerificationError::MilestoneAlreadyExists);
        }

        // ── Index continuity (1-based, matching commit_approved_milestone) ───
        let counter_key = DataKey::MilestoneCounter(player_id);
        let current_count: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0u32);
        let expected_index =
            safe_add_u32(current_count, 1).map_err(|_| VerificationError::Overflow)?;
        if milestone_index != expected_index {
            return Err(VerificationError::MilestoneNotFound);
        }

        // ── EvidenceUsed uniqueness ───────────────────────────────────────────
        let ev_key = DataKey::EvidenceUsed(milestone.evidence_hash.clone());
        if let Some((existing_player, existing_index)) = env
            .storage()
            .persistent()
            .get::<DataKey, (u64, u32)>(&ev_key)
        {
            if existing_player != player_id || existing_index != milestone_index {
                return Err(VerificationError::DuplicateEvidence);
            }
            // Already mapped to this exact slot — idempotent; fall through to
            // write the primary record (checked absent above).
        }

        // ── Write primary Milestone record ────────────────────────────────────
        env.storage().persistent().set(&ms_key, &milestone);
        env.storage()
            .persistent()
            .extend_ttl(&ms_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Register EvidenceUsed ─────────────────────────────────────────────
        env.storage()
            .persistent()
            .set(&ev_key, &(player_id, milestone_index));
        env.storage()
            .persistent()
            .extend_ttl(&ev_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Increment MilestoneCounter(player_id) ─────────────────────────────
        env.storage()
            .persistent()
            .set(&counter_key, &milestone_index);
        env.storage()
            .persistent()
            .extend_ttl(&counter_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Increment TotalMilestoneCount (instance storage) ───────────────────
        let total_key = DataKey::TotalMilestoneCount;
        let total: u32 = env.storage().instance().get(&total_key).unwrap_or(0u32);
        let new_total = safe_add_u32(total, 1).map_err(|_| VerificationError::Overflow)?;
        env.storage().instance().set(&total_key, &new_total);

        // ── Increment ValidatorMilestoneCount(validator) ──────────────────────
        let vmc_key = DataKey::ValidatorMilestoneCount(validator.clone());
        let vmc: u32 = env.storage().persistent().get(&vmc_key).unwrap_or(0u32);
        let new_vmc = safe_add_u32(vmc, 1).map_err(|_| VerificationError::Overflow)?;
        env.storage().persistent().set(&vmc_key, &new_vmc);
        env.storage()
            .persistent()
            .extend_ttl(&vmc_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Increment ValidatorPlayerMilestoneCount(validator, player_id) ─────
        let vpmc_key = DataKey::ValidatorPlayerMilestoneCount(validator.clone(), player_id);
        let vpmc: u32 = env.storage().persistent().get(&vpmc_key).unwrap_or(0u32);
        let new_vpmc = safe_add_u32(vpmc, 1).map_err(|_| VerificationError::Overflow)?;
        env.storage().persistent().set(&vpmc_key, &new_vpmc);
        env.storage()
            .persistent()
            .extend_ttl(&vpmc_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Update ring-buffer global milestone index ─────────────────────────
        // Mirror the same O(1) write path used by commit_approved_milestone:
        // read write_head, compute slot, write slot entry, increment write_head.
        // Because admin_seed_milestone must be idempotent and seeds in index
        // order (validated above), each seed call is a fresh sequential write.
        let wh_key = DataKey::GlobalMilestoneWriteHead;
        let write_head: u32 = env.storage().instance().get(&wh_key).unwrap_or(0u32);
        let slot = write_head % MAX_GLOBAL_MILESTONE_INDEX;
        env.storage().persistent().set(
            &DataKey::GlobalMilestoneSlot(slot),
            &GlobalMilestoneEntry {
                player_id,
                milestone_index,
            },
        );
        env.storage().persistent().extend_ttl(
            &DataKey::GlobalMilestoneSlot(slot),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        env.storage().instance().set(
            &wh_key,
            &(safe_add_u32(write_head, 1).map_err(|_| VerificationError::Overflow)?),
        );

        // ── Update ValidatorMilestones(validator) ─────────────────────────────
        let vm_key = DataKey::ValidatorMilestones(validator.clone());
        let mut vm: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&vm_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !vm
            .iter()
            .any(|r| r.player_id == player_id && r.milestone_index == milestone_index)
        {
            vm.push_back(MilestoneRef {
                player_id,
                milestone_index,
            });
            env.storage().persistent().set(&vm_key, &vm);
            env.storage()
                .persistent()
                .extend_ttl(&vm_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        // ── Update ValidatorPlayers(validator) ────────────────────────────────
        let vp_key = DataKey::ValidatorPlayers(validator.clone());
        let mut vp: Vec<u64> = env
            .storage()
            .persistent()
            .get(&vp_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !vp.iter().any(|id| id == player_id) {
            vp.push_back(player_id);
            env.storage().persistent().set(&vp_key, &vp);
            env.storage()
                .persistent()
                .extend_ttl(&vp_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        Ok(())
    }

    /// Seed a historical `MilestoneDispute` from a prior contract deployment.
    ///
    /// This is the migration entrypoint for **in-flight disputes**
    /// (MIGRATION_GAPS row 7).  It reconstructs all dispute-related derived
    /// state:
    ///
    /// - `MilestoneDispute(player_id, milestone_index)` — the dispute record
    /// - `PlayerDisputes(player_id)` — per-player index of disputed milestones
    /// - `OpenDisputeIndex` — global unresolved index (if `!dispute.resolved`)
    /// - `ActiveDisputesCount` — running count of open disputes
    ///
    /// ## Idempotency
    ///
    /// Keyed on `(player_id, milestone_index)`.  Identical replay → no-op.
    /// Conflicting content → `DisputeAlreadyExists`.
    pub fn admin_seed_dispute(
        env: Env,
        player_id: u64,
        milestone_index: u32,
        dispute: MilestoneDispute,
    ) -> Result<(), VerificationError> {
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
        Self::require_initialized(&env)?;
        Self::require_migration_active(&env)?;

        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);

        // ── Idempotency ───────────────────────────────────────────────────────
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, MilestoneDispute>(&dispute_key)
        {
            let identical = existing.player_id == dispute.player_id
                && existing.milestone_index == dispute.milestone_index
                && existing.reason == dispute.reason
                && existing.disputed_at == dispute.disputed_at
                && existing.resolved == dispute.resolved
                && existing.upheld == dispute.upheld;
            if identical {
                return Ok(());
            }
            return Err(VerificationError::DisputeAlreadyExists);
        }

        // ── Write dispute record ──────────────────────────────────────────────
        env.storage().persistent().set(&dispute_key, &dispute);
        env.storage()
            .persistent()
            .extend_ttl(&dispute_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        // ── Update PlayerDisputes(player_id) ──────────────────────────────────
        let pd_key = DataKey::PlayerDisputes(player_id);
        let mut pd: Vec<u32> = env
            .storage()
            .persistent()
            .get(&pd_key)
            .unwrap_or_else(|| Vec::new(&env));
        if !pd.iter().any(|idx| idx == milestone_index) {
            pd.push_back(milestone_index);
            env.storage().persistent().set(&pd_key, &pd);
            env.storage()
                .persistent()
                .extend_ttl(&pd_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        }

        // ── Update OpenDisputeIndex + ActiveDisputesCount (if unresolved) ─────
        if !dispute.resolved {
            let odi_key = DataKey::OpenDisputeIndex;
            let mut odi: Vec<(u64, u32)> = env
                .storage()
                .persistent()
                .get(&odi_key)
                .unwrap_or_else(|| Vec::new(&env));
            let already_open = odi
                .iter()
                .any(|(pid, midx)| pid == player_id && midx == milestone_index);
            if !already_open {
                odi.push_back((player_id, milestone_index));
                env.storage().persistent().set(&odi_key, &odi);
                env.storage().persistent().extend_ttl(
                    &odi_key,
                    PERSISTENT_TTL_MIN,
                    PERSISTENT_TTL_MAX,
                );

                let adc_key = DataKey::ActiveDisputesCount;
                let adc: u32 = env.storage().instance().get(&adc_key).unwrap_or(0u32);
                let new_adc = safe_add_u32(adc, 1).map_err(|_| VerificationError::Overflow)?;
                env.storage().instance().set(&adc_key, &new_adc);
            }
        }

        Ok(())
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
        impact_score: u32,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        player_wallet.require_auth();

        // Verify the milestone exists
        let milestone: Milestone = env
            .storage()
            .persistent()
            .get::<DataKey, Milestone>(&DataKey::Milestone(player_id, milestone_index))
            .ok_or(VerificationError::MilestoneNotFound)?;

        // Verify the caller's wallet actually corresponds to player_id
        // by making a cross-contract call to the registration contract.
        // This replaces the previous tautological check (milestone.player_id
        // could never differ from player_id) with a real authorization gate.
        let reg_addr = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegistrationContract)
            .ok_or(VerificationError::RegistrationCallFailed)?;
        let args: Vec<Val> = (player_id,).into_val(&env);
        let profile: RegPlayerProfile = env
            .try_invoke_contract::<RegPlayerProfile, VerificationError>(
                &reg_addr,
                &Symbol::new(&env, "get_player"),
                args,
            )
            .map_err(|_| VerificationError::RegistrationCallFailed)?
            .map_err(|_| VerificationError::RegistrationCallFailed)?;
        if profile.wallet != player_wallet {
            return Err(VerificationError::Unauthorized);
        }

        // Check if dispute already exists
        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);
        if env.storage().persistent().has(&dispute_key) {
            return Err(VerificationError::InvalidInput);
        }

        // Snapshot the jury configuration at filing time so later admin
        // changes to JuryConfig cannot alter this dispute's rules mid-vote.
        let jury_config = Self::get_jury_config_internal(&env);
        let jury_required = impact_score >= jury_config.impact_threshold;
        let now = env.ledger().timestamp();
        let voting_deadline = if jury_required {
            safe_add_u64(now, jury_config.voting_window_secs)
                .map_err(|_| VerificationError::Overflow)?
        } else {
            0u64
        };

        let dispute = MilestoneDispute {
            player_id,
            milestone_index,
            reason: reason.clone(),
            disputed_at: now,
            resolved: false,
            upheld: false,
            impact_score,
            jury_required,
            quorum: jury_config.quorum,
            voting_deadline,
            votes_for: 0,
            votes_against: 0,
        };

        // Keep the approver address from the milestone for conflict-of-interest checks.
        // This is read during cast_dispute_vote via the Milestone storage record directly.
        // Suppress the unused-variable warning — `milestone` was fetched above for
        // existence validation; the approver address is re-read from storage in
        // cast_dispute_vote to avoid re-serialising the full record here.
        let _ = &milestone.validator;

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
            &safe_add_u32(count, 1).map_err(|_| VerificationError::Overflow)?,
        );

        // Maintain the global open-dispute index so list_disputes_page can
        // enumerate unresolved disputes without knowing every (player_id, index) pair.
        let open_index_key = DataKey::OpenDisputeIndex;
        let mut open_index: Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&open_index_key)
            .unwrap_or_else(|| Vec::new(&env));
        open_index.push_back((player_id, milestone_index));
        env.storage().persistent().set(&open_index_key, &open_index);
        env.storage().persistent().extend_ttl(
            &open_index_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::milestone_disputed(&env, &player_wallet, player_id, milestone_index, &reason);
        Ok(())
    }

    /// Resolve a filed milestone dispute (admin only).
    ///
    /// This marks the dispute as resolved and records whether the admin upheld
    /// it. It does not roll back player progress; that corrective workflow is
    /// intentionally handled separately.
    ///
    /// Returns `DisputeRequiresJury` for disputes that were routed to the jury
    /// path at filing time (`jury_required == true`) — those must be finalized
    /// via `tally_dispute`.
    pub fn resolve_dispute(
        env: Env,
        player_id: u64,
        milestone_index: u32,
        upheld: bool,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
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

        // Jury-required disputes must be finalized via tally_dispute, not by admin.
        if dispute.jury_required {
            return Err(VerificationError::DisputeRequiresJury);
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
            &safe_sub_u32(count, 1).map_err(|_| VerificationError::Overflow)?,
        );

        // Remove this dispute from the global open-dispute index so it no
        // longer appears in list_disputes_page results.
        let open_index_key = DataKey::OpenDisputeIndex;
        let open_index: Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&open_index_key)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_index: Vec<(u64, u32)> = Vec::new(&env);
        for i in 0..open_index.len() {
            let entry = open_index.get(i).unwrap();
            if entry != (player_id, milestone_index) {
                new_index.push_back(entry);
            }
        }
        env.storage().persistent().set(&open_index_key, &new_index);
        if !new_index.is_empty() {
            env.storage().persistent().extend_ttl(
                &open_index_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        events::dispute_resolved(&env, &admin, player_id, milestone_index, upheld);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Jury escalation system (issue #1036)
    // -------------------------------------------------------------------------

    /// Internal helper: read JuryConfig from instance storage, returning
    /// defaults (threshold=100, quorum=3, voting_window_secs=604800) if unset.
    fn get_jury_config_internal(env: &Env) -> JuryConfig {
        env.storage()
            .instance()
            .get::<DataKey, JuryConfig>(&DataKey::JuryConfig)
            .unwrap_or(JuryConfig {
                impact_threshold: 100,
                quorum: 3,
                voting_window_secs: 604800,
            })
    }

    /// Configure the jury escalation parameters (admin only).
    ///
    /// - `impact_threshold`: disputes with `impact_score >= threshold` are jury-routed.
    ///   Default: 100.
    /// - `quorum`: minimum distinct validator votes required for a jury outcome.
    ///   Default: 3.
    /// - `voting_window_secs`: seconds after filing before the voting window closes.
    ///   Default: 604800 (7 days).
    ///
    /// This only affects disputes filed *after* this call — in-flight disputes
    /// snapshot the parameters at filing time.
    pub fn set_jury_config(
        env: Env,
        impact_threshold: u32,
        quorum: u32,
        voting_window_secs: u64,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;

        let config = JuryConfig {
            impact_threshold,
            quorum,
            voting_window_secs,
        };
        env.storage().instance().set(&DataKey::JuryConfig, &config);
        Ok(())
    }

    /// Return the current `JuryConfig` (impact_threshold, quorum, voting_window_secs).
    /// If never set by admin, returns the defaults (100 / 3 / 604800).
    pub fn get_jury_config(env: Env) -> JuryConfig {
        Self::get_jury_config_internal(&env)
    }

    /// Cast a validator vote on a jury-required milestone dispute.
    ///
    /// Eligibility rules (all four must hold):
    /// 1. Validator is registered and active.
    /// 2. Validator is not the original approver of the disputed milestone
    ///    (conflict of interest).
    /// 3. Validator has not already voted on this dispute.
    /// 4. Dispute is jury-required, unresolved, and the voting window is still open.
    ///
    /// Emits `dispute_vote_cast` on success.
    pub fn cast_dispute_vote(
        env: Env,
        validator: Address,
        player_id: u64,
        milestone_index: u32,
        for_upheld: bool,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        validator.require_auth();

        // Rule 1: validator must be registered and active.
        let val_record: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(validator.clone()))
            .ok_or(VerificationError::ValidatorNotFound)?;
        if !val_record.active {
            return Err(VerificationError::ValidatorInactive);
        }

        // Load the dispute.
        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);
        let mut dispute: MilestoneDispute = env
            .storage()
            .persistent()
            .get(&dispute_key)
            .ok_or(VerificationError::MilestoneNotFound)?;

        // Rule 4a: must be jury-required.
        if !dispute.jury_required {
            return Err(VerificationError::NotJuryDispute);
        }

        // Rule 4b: must be unresolved.
        if dispute.resolved {
            return Err(VerificationError::DisputeAlreadyResolved);
        }

        // Rule 4c: voting window must still be open.
        let now = env.ledger().timestamp();
        if now >= dispute.voting_deadline {
            return Err(VerificationError::VotingWindowClosed);
        }

        // Rule 2: validator must not be the original milestone approver.
        let milestone: Milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, milestone_index))
            .ok_or(VerificationError::MilestoneNotFound)?;
        if milestone.validator == validator {
            return Err(VerificationError::ConflictOfInterest);
        }

        // Rule 3: validator must not have already voted.
        let vote_key = DataKey::DisputeVote(player_id, milestone_index, validator.clone());
        if env.storage().persistent().has(&vote_key) {
            return Err(VerificationError::AlreadyVoted);
        }

        // Record the individual vote for audit trail.
        let vote = DisputeVote {
            validator: validator.clone(),
            for_upheld,
            voted_at: now,
        };
        env.storage().persistent().set(&vote_key, &vote);
        env.storage().persistent().extend_ttl(
            &vote_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Update running vote tallies on the dispute record.
        if for_upheld {
            dispute.votes_for = safe_add_u32(dispute.votes_for, 1)
                .map_err(|_| VerificationError::Overflow)?;
        } else {
            dispute.votes_against = safe_add_u32(dispute.votes_against, 1)
                .map_err(|_| VerificationError::Overflow)?;
        }
        env.storage().persistent().set(&dispute_key, &dispute);
        env.storage().persistent().extend_ttl(
            &dispute_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        // Update the per-dispute vote count index.
        let count_key = DataKey::DisputeVoteCount(player_id, milestone_index);
        let vote_count: u32 = env
            .storage()
            .persistent()
            .get(&count_key)
            .unwrap_or(0u32);
        let new_count = safe_add_u32(vote_count, 1)
            .map_err(|_| VerificationError::Overflow)?;
        env.storage().persistent().set(&count_key, &new_count);
        env.storage().persistent().extend_ttl(
            &count_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        events::dispute_vote_cast(&env, player_id, milestone_index, &validator, for_upheld);
        Ok(())
    }

    /// Finalize a jury-required milestone dispute.
    ///
    /// `tally_dispute` succeeds when the dispute is jury-required, unresolved,
    /// and either:
    /// - **Early close**: total votes >= quorum AND votes_for ≠ votes_against (clear majority), or
    /// - **Deadline passed**: current time >= voting_deadline (majority rules).
    ///   If below quorum at deadline, resolves upheld=false.
    ///
    /// Tie-break: if votes_for == votes_against, dispute is rejected (upheld=false).
    ///
    /// This function is callable by anyone — there is no admin requirement.
    /// Emits `dispute_tallied` and removes the dispute from the open index.
    pub fn tally_dispute(
        env: Env,
        player_id: u64,
        milestone_index: u32,
    ) -> Result<(), VerificationError> {
        Self::bump_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;

        let dispute_key = DataKey::MilestoneDispute(player_id, milestone_index);
        let mut dispute: MilestoneDispute = env
            .storage()
            .persistent()
            .get(&dispute_key)
            .ok_or(VerificationError::MilestoneNotFound)?;

        // Must be jury-required.
        if !dispute.jury_required {
            return Err(VerificationError::NotJuryDispute);
        }

        // Must be unresolved.
        if dispute.resolved {
            return Err(VerificationError::DisputeAlreadyResolved);
        }

        let now = env.ledger().timestamp();
        let total_votes = safe_add_u32(dispute.votes_for, dispute.votes_against)
            .map_err(|_| VerificationError::Overflow)?;
        let deadline_passed = now >= dispute.voting_deadline;

        // Determine whether tally is allowed right now.
        let early_close = total_votes >= dispute.quorum && dispute.votes_for != dispute.votes_against;

        if deadline_passed {
            // Deadline path: tally regardless of vote count.
            // If below quorum, resolve not-upheld.
        } else if early_close {
            // Clear majority reached quorum — finalize early.
        } else if total_votes >= dispute.quorum && dispute.votes_for == dispute.votes_against {
            // Tied at quorum — can only resolve after deadline.
            return Err(VerificationError::VotingWindowOpen);
        } else {
            // Window still open and quorum not yet reached.
            return Err(VerificationError::QuorumNotReached);
        }

        // Determine outcome.
        let upheld = if total_votes < dispute.quorum {
            // Below quorum at deadline: reject.
            false
        } else if dispute.votes_for > dispute.votes_against {
            true
        } else {
            // Includes tie case (votes_for == votes_against) → reject.
            false
        };

        dispute.resolved = true;
        dispute.upheld = upheld;
        env.storage().persistent().set(&dispute_key, &dispute);

        // Decrement active disputes counter.
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ActiveDisputesCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::ActiveDisputesCount,
            &safe_sub_u32(count, 1).map_err(|_| VerificationError::Overflow)?,
        );

        // Remove from the global open-dispute index.
        let open_index_key = DataKey::OpenDisputeIndex;
        let open_index: Vec<(u64, u32)> = env
            .storage()
            .persistent()
            .get(&open_index_key)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_index: Vec<(u64, u32)> = Vec::new(&env);
        for i in 0..open_index.len() {
            let entry = open_index.get(i).unwrap();
            if entry != (player_id, milestone_index) {
                new_index.push_back(entry);
            }
        }
        env.storage()
            .persistent()
            .set(&open_index_key, &new_index);
        if !new_index.is_empty() {
            env.storage().persistent().extend_ttl(
                &open_index_key,
                PERSISTENT_TTL_MIN,
                PERSISTENT_TTL_MAX,
            );
        }

        events::dispute_tallied(
            &env,
            player_id,
            milestone_index,
            upheld,
            dispute.votes_for,
            dispute.votes_against,
        );
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

    // -------------------------------------------------------------------------
    // Revocation cascade re-review (issue #1039)
    // -------------------------------------------------------------------------

    /// Returns `true` if the milestone at `(player_id, milestone_index)` is
    /// currently flagged as pending re-review due to a for-cause validator
    /// revocation cascade.  Returns `false` if it has never been flagged or
    /// has already been cleared by `rereview_milestone`.
    pub fn is_milestone_flagged(env: Env, player_id: u64, milestone_index: u32) -> bool {
        // Read the legacy key first so flags written by an older contract
        // version remain visible after an upgrade.
        if env
            .storage()
            .persistent()
            .has(&DataKey::MilestonePendingReReview(
                player_id,
                milestone_index,
            ))
        {
            return true;
        }

        let milestone: Milestone = match env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, milestone_index))
        {
            Some(value) => value,
            None => return false,
        };
        let wallet = milestone.validator;
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MilestonePendingReReviewCount(wallet.clone()))
            .unwrap_or(0);
        let page_count = (count + 49) / 50;
        for page_index in 0..page_count {
            if let Some(page) = env
                .storage()
                .persistent()
                .get::<DataKey, Vec<MilestoneRef>>(&DataKey::MilestonePendingReReviewPage(
                    wallet.clone(),
                    page_index,
                ))
            {
                for i in 0..page.len() {
                    let reference = page.get(i).unwrap();
                    if reference.player_id == player_id
                        && reference.milestone_index == milestone_index
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Clear a pending re-review flag on a milestone after independently
    /// confirming the underlying achievement.
    ///
    /// Caller (`reviewer`) must be a currently-active validator (not
    /// necessarily the original approver of the milestone).  Emits
    /// `milestone_flag_cleared` on success.
    ///
    /// Returns:
    /// - `MilestoneNotFound` if the milestone does not exist.
    /// - `NotEligibleToReReview` if the caller is not a currently-active
    ///   validator.
    /// - `MilestoneNotFlagged` if the milestone is not currently flagged.
    pub fn rereview_milestone(
        env: Env,
        reviewer: Address,
        player_id: u64,
        milestone_index: u32,
    ) -> Result<(), VerificationError> {
        reviewer.require_auth();

        // Milestone must exist.
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Milestone(player_id, milestone_index))
        {
            return Err(VerificationError::MilestoneNotFound);
        }

        // Reviewer must be a currently-active validator.
        let validator_status = Self::get_validator_status(env.clone(), reviewer.clone());
        if validator_status != ValidatorStatus::Active {
            return Err(VerificationError::NotEligibleToReReview);
        }

        // Clear a legacy per-milestone flag when present.
        let legacy_flag_key = DataKey::MilestonePendingReReview(player_id, milestone_index);
        if env.storage().persistent().has(&legacy_flag_key) {
            env.storage().persistent().remove(&legacy_flag_key);
            events::milestone_flag_cleared(&env, &reviewer, player_id, milestone_index);
            return Ok(());
        }

        // New cascades store compact references in pages scoped to the
        // validator that originally approved this milestone.
        let milestone: Milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(player_id, milestone_index))
            .ok_or(VerificationError::MilestoneNotFound)?;
        let wallet = milestone.validator;
        let count_key = DataKey::MilestonePendingReReviewCount(wallet.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let page_count = (count + 49) / 50;
        let mut cleared = false;
        for page_index in 0..page_count {
            let page_key = DataKey::MilestonePendingReReviewPage(wallet.clone(), page_index);
            let page: Vec<MilestoneRef> = env
                .storage()
                .persistent()
                .get(&page_key)
                .unwrap_or_else(|| Vec::new(&env));
            let mut replacement: Vec<MilestoneRef> = Vec::new(&env);
            for i in 0..page.len() {
                let reference = page.get(i).unwrap();
                if reference.player_id == player_id && reference.milestone_index == milestone_index
                {
                    cleared = true;
                } else {
                    replacement.push_back(reference);
                }
            }
            if cleared {
                env.storage().persistent().set(&page_key, &replacement);
                env.storage().persistent().extend_ttl(
                    &page_key,
                    PERSISTENT_TTL_MIN,
                    PERSISTENT_TTL_MAX,
                );
                break;
            }
        }

        if !cleared {
            return Err(VerificationError::MilestoneNotFlagged);
        }
        env.storage()
            .persistent()
            .set(&count_key, &count.saturating_sub(1));
        env.storage()
            .persistent()
            .extend_ttl(&count_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);
        events::milestone_flag_cleared(&env, &reviewer, player_id, milestone_index);
        Ok(())
    }

    /// Return the stored `RevocationRecord` for a validator wallet, if any.
    ///
    /// Returns `None` if the validator has never been revoked via the new
    /// severity-aware `revoke_validator` path (i.e. old routine revocations
    /// performed before this feature shipped will not have a record).
    pub fn get_revocation_record(env: Env, wallet: Address) -> Option<RevocationRecord> {
        env.storage()
            .persistent()
            .get(&DataKey::RevocationRecord(wallet))
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

    fn require_migration_active(env: &Env) -> Result<(), VerificationError> {
        let active = env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::MigrationActive)
            .unwrap_or(false);
        if !active {
            return Err(VerificationError::MigrationNotActive);
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

    /// Enforce the per-wallet validator registration cooldown.
    ///
    /// Reads the last-sent timestamp stored under `last_sent_key`.  If a
    /// timestamp is present and the current ledger time is before
    /// `last_sent + cooldown_secs`, returns `RegistrationCooldown`.
    /// A cooldown of 0 disables the check entirely.
    fn enforce_reg_cooldown(env: &Env, last_sent_key: &DataKey) -> Result<(), VerificationError> {
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
                safe_add_u64(last_sent, cooldown_secs).map_err(|_| VerificationError::Overflow)?;
            if now < next_allowed {
                return Err(VerificationError::RegistrationCooldown);
            }
        }
        Ok(())
    }

    /// Check that approve_milestone is not paused (function-scoped circuit breaker).
    /// Independent of the whole-contract pause flag.
    fn require_approve_milestone_not_paused(env: &Env) -> Result<(), VerificationError> {
        if env
            .storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::PausedApproveMilestone)
            .unwrap_or(false)
        {
            return Err(VerificationError::ApproveMilestonePaused);
        }
        Ok(())
    }

    /// Retroactively invalidate `wallet`'s contribution to every still-open
    /// (sub-threshold) pending attestation claim it has voted on, called
    /// from `revoke_validator` / `batch_revoke_validators`.
    ///
    /// Bounded to `MAX_PENDING_VOTES_PER_VALIDATOR` — see
    /// `DataKey::ValidatorPendingVotes` — so this never scans more than a
    /// small constant number of entries regardless of how many claims exist
    /// contract-wide or how long the validator has been registered. Returns
    /// the number of claims actually decremented, for the emitted event.
    fn invalidate_pending_votes_for_validator(env: &Env, wallet: &Address) -> u32 {
        let pending_votes_key = DataKey::ValidatorPendingVotes(wallet.clone());
        let refs: Vec<PendingVoteRef> = env
            .storage()
            .persistent()
            .get(&pending_votes_key)
            .unwrap_or_else(|| Vec::new(env));

        // The revoked validator's region/affiliation, used to decrement the
        // per-claim attester diversity tallies so the gates keep evaluating
        // over the true attesting set (revoked votes no longer count). The
        // `Validator` record survives revocation (only `active` flips), so it
        // is still readable here; if it is ever absent, the tallies are left
        // as-is rather than crashing the admin action.
        let revoked_validator: Option<Validator> = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(wallet.clone()));

        let mut invalidated = 0u32;
        for i in 0..refs.len() {
            let vref = refs.get(i).unwrap();
            let claim_key =
                DataKey::PendingMilestoneClaim(vref.player_id, vref.evidence_hash.clone());
            if let Some(mut claim) = env
                .storage()
                .persistent()
                .get::<DataKey, PendingMilestoneClaim>(&claim_key)
            {
                // Only claims still in the same round the vote was cast for
                // are live; a claim that already moved to a later round (via
                // expiry) already discarded this vote implicitly.
                if claim.round == vref.round && claim.vote_count > 0 {
                    claim.vote_count -= 1;
                    // Remove this vote's contribution from the attester
                    // diversity tallies, keeping them in lockstep with the
                    // vote count so the gates see the true attesting set.
                    if let Some(v) = &revoked_validator {
                        Self::decrement_diversity_value(&mut claim.attester_regions, &v.region);
                        Self::decrement_diversity_value(
                            &mut claim.attester_affiliations,
                            &v.affiliation,
                        );
                    }
                    env.storage().persistent().set(&claim_key, &claim);
                    let vote_key = DataKey::PendingMilestoneVote(
                        vref.player_id,
                        vref.evidence_hash.clone(),
                        vref.round,
                        wallet.clone(),
                    );
                    env.storage().persistent().remove(&vote_key);
                    invalidated += 1;
                }
            }
        }

        if !refs.is_empty() {
            env.storage().persistent().remove(&pending_votes_key);
        }
        invalidated
    }

    /// Record one more accepted attestation's region/affiliation in the
    /// claim's per-(value, count) diversity tallies (see
    /// `PendingMilestoneClaim::attester_regions` / `attester_affiliations`).
    fn bump_attester_diversity(claim: &mut PendingMilestoneClaim, validator: &Validator) {
        Self::bump_diversity_value(&mut claim.attester_regions, &validator.region);
        Self::bump_diversity_value(&mut claim.attester_affiliations, &validator.affiliation);
    }

    /// Increment the count for `value` in a per-(value, count) tally, pushing
    /// a fresh `(value, 1)` entry the first time it is seen.
    fn bump_diversity_value(entries: &mut Vec<(String, u32)>, value: &String) {
        for i in 0..entries.len() {
            let (existing, count) = entries.get(i).unwrap();
            if existing == *value {
                entries.set(i, (existing.clone(), count.saturating_add(1)));
                return;
            }
        }
        entries.push_back((value.clone(), 1));
    }

    /// Decrement the count for `value` in a per-(value, count) tally, removing
    /// the entry entirely when its count reaches zero (that value no longer
    /// appears in the attesting set).
    fn decrement_diversity_value(entries: &mut Vec<(String, u32)>, value: &String) {
        for i in 0..entries.len() {
            let (existing, count) = entries.get(i).unwrap();
            if existing == *value {
                if count <= 1 {
                    entries.remove(i);
                } else {
                    entries.set(i, (existing.clone(), count - 1));
                }
                return;
            }
        }
    }

    /// Distinct values with a positive count in a per-(value, count) tally —
    /// i.e. the distinct regions/affiliations present in the attesting set.
    fn distinct_diversity_values(entries: &Vec<(String, u32)>) -> Vec<String> {
        let mut distinct: Vec<String> = Vec::new(entries.env());
        for i in 0..entries.len() {
            let (value, count) = entries.get(i).unwrap();
            if *count > 0 && !distinct.contains(value) {
                distinct.push_back(value.clone());
            }
        }
        distinct
    }

    /// Shared milestone commit used by `approve_milestone`,
    /// `submit_attested_milestone`, and `attest_milestone` (on threshold
    /// cross). Caller must already have authenticated the validator and
    /// validated description/evidence/category constraints.
    ///
    /// `attester_diversity` is `Some` only for a genuine k-of-n threshold
    /// cross (`attest_milestone` with threshold > 1): the distinct
    /// regions/affiliations of the validators whose votes reached the
    /// threshold. The region-quorum and affiliation-diversity gates then
    /// evaluate against THAT set, so a threshold met entirely by validators
    /// from a single region or single affiliation never advances the level —
    /// even when the player's accumulated milestone history already spans
    /// the required diversity. `None` (single-validator paths) keeps the
    /// accumulated per-player evaluation. In both cases the milestone itself
    /// is always recorded; only the level advance is gated.
    fn commit_approved_milestone(
        env: &Env,
        validator_wallet: &Address,
        player_id: u64,
        description: String,
        evidence_hash: String,
        attester_diversity: Option<AttesterDiversity>,
    ) -> Result<u32, VerificationError> {
        let evidence_used_key = DataKey::EvidenceUsed(evidence_hash.clone());
        if env.storage().persistent().has(&evidence_used_key) {
            return Err(VerificationError::DuplicateEvidence);
        }

        let vp_key = DataKey::ValidatorPlayerMilestoneCount(validator_wallet.clone(), player_id);
        let vp_count: u32 = env.storage().persistent().get(&vp_key).unwrap_or(0u32);
        if vp_count >= MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR {
            return Err(VerificationError::MilestoneLimitExceeded);
        }

        let counter_key = DataKey::MilestoneCounter(player_id);
        let index: u32 = env.storage().persistent().get(&counter_key).unwrap_or(0u32);
        let next_index = safe_add_u32(index, 1).map_err(|_| VerificationError::Overflow)?;

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
        env.storage().persistent().extend_ttl(
            &DataKey::Milestone(player_id, next_index),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        env.storage().persistent().set(&counter_key, &next_index);
        env.storage()
            .persistent()
            .extend_ttl(&counter_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        env.storage()
            .persistent()
            .set(&evidence_used_key, &(player_id, next_index));
        env.storage().persistent().extend_ttl(
            &evidence_used_key,
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );

        let val_key = DataKey::ValidatorMilestoneCount(validator_wallet.clone());
        let val_count: u32 = env.storage().persistent().get(&val_key).unwrap_or(0u32);
        env.storage().persistent().set(
            &val_key,
            &(safe_add_u32(val_count, 1).map_err(|_| VerificationError::Overflow)?),
        );
        env.storage()
            .persistent()
            .extend_ttl(&val_key, PERSISTENT_TTL_MIN, PERSISTENT_TTL_MAX);

        env.storage().persistent().set(
            &vp_key,
            &(safe_add_u32(vp_count, 1).map_err(|_| VerificationError::Overflow)?),
        );

        let vp_index_key = DataKey::ValidatorPlayers(validator_wallet.clone());
        let mut vp_players: Vec<u64> = env
            .storage()
            .persistent()
            .get(&vp_index_key)
            .unwrap_or_else(|| Vec::new(env));
        if !vp_players.contains(player_id) {
            vp_players.push_back(player_id);
            env.storage().persistent().set(&vp_index_key, &vp_players);
        }

        let total: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalMilestoneCount)
            .unwrap_or(0u32);
        env.storage().instance().set(
            &DataKey::TotalMilestoneCount,
            &(safe_add_u32(total, 1).map_err(|_| VerificationError::Overflow)?),
        );

        // ── O(1) ring-buffer write ────────────────────────────────────────────
        // Reads one instance key (write_head), writes one persistent slot, and
        // writes one instance key (write_head+1). Cost is constant regardless
        // of how many entries currently live in the ring or have ever been
        // written — there is no Vec to read, shift, or serialize.
        let write_head: u32 = env
            .storage()
            .instance()
            .get(&DataKey::GlobalMilestoneWriteHead)
            .unwrap_or(0u32);
        let slot = write_head % MAX_GLOBAL_MILESTONE_INDEX;
        env.storage().persistent().set(
            &DataKey::GlobalMilestoneSlot(slot),
            &GlobalMilestoneEntry {
                player_id,
                milestone_index: next_index,
            },
        );
        env.storage().persistent().extend_ttl(
            &DataKey::GlobalMilestoneSlot(slot),
            PERSISTENT_TTL_MIN,
            PERSISTENT_TTL_MAX,
        );
        env.storage().instance().set(
            &DataKey::GlobalMilestoneWriteHead,
            &(safe_add_u32(write_head, 1).map_err(|_| VerificationError::Overflow)?),
        );

        let validator_milestones_key = DataKey::ValidatorMilestones(validator_wallet.clone());
        let mut validator_milestones: Vec<MilestoneRef> = env
            .storage()
            .persistent()
            .get(&validator_milestones_key)
            .unwrap_or_else(|| Vec::new(env));
        validator_milestones.push_back(MilestoneRef {
            player_id,
            milestone_index: next_index,
        });
        env.storage()
            .persistent()
            .set(&validator_milestones_key, &validator_milestones);

        events::milestone_approved(
            env,
            player_id,
            validator_wallet,
            next_index,
            &description,
            &evidence_hash,
        );

        let validator: Validator = env
            .storage()
            .persistent()
            .get(&DataKey::Validator(validator_wallet.clone()))
            .unwrap();

        // ── Anti-collusion accumulators ────────────────────────────────────
        // Track the distinct validator affiliations/regions that have
        // contributed a milestone for this player. For a k-of-n approval the
        // contributing set is the whole attesting set (every validator whose
        // vote counted toward the threshold); for a single-validator approval
        // it is just the committing validator. These accumulated sets feed
        // the diversity gates for the single-validator paths and keep the
        // "milestones from validators in N distinct regions must exist for
        // the player" semantics of `set_min_region_quorum` / `DiversityConfig`.
        let mut player_affiliations: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerAffiliations(player_id))
            .unwrap_or_else(|| Vec::new(env));

        let mut affiliations_updated = false;
        match &attester_diversity {
            Some(div) => {
                for i in 0..div.affiliations.len() {
                    let affiliation = div.affiliations.get(i).unwrap();
                    if !player_affiliations.contains(affiliation) {
                        player_affiliations.push_back(affiliation.clone());
                        affiliations_updated = true;
                    }
                }
            }
            None => {
                if !player_affiliations.contains(&validator.affiliation) {
                    player_affiliations.push_back(validator.affiliation.clone());
                    affiliations_updated = true;
                }
            }
        }
        if affiliations_updated {
            env.storage().persistent().set(
                &DataKey::PlayerAffiliations(player_id),
                &player_affiliations,
            );
        }

        let mut player_regions: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerRegions(player_id))
            .unwrap_or_else(|| Vec::new(env));

        let mut regions_updated = false;
        match &attester_diversity {
            Some(div) => {
                for i in 0..div.regions.len() {
                    let region = div.regions.get(i).unwrap();
                    if !player_regions.contains(region) {
                        player_regions.push_back(region.clone());
                        regions_updated = true;
                    }
                }
            }
            None => {
                if !player_regions.contains(&validator.region) {
                    player_regions.push_back(validator.region.clone());
                    regions_updated = true;
                }
            }
        }
        if regions_updated {
            env.storage()
                .persistent()
                .set(&DataKey::PlayerRegions(player_id), &player_regions);
        }

        // ── Anti-collusion gates ───────────────────────────────────────────
        // `advance_level` is skipped when the milestone's approving set fails
        // the affiliation-diversity and/or region-quorum requirements. Which
        // set the gates run against depends on how the milestone was approved:
        //
        // - k-of-n (`attester_diversity` is Some): the attesting validator
        //   set — every validator whose vote contributed to reaching the
        //   threshold. A threshold met by validators all from one region or
        //   one affiliation can never smuggle the level advance through
        //   accumulated history from other milestones.
        //
        // - single-validator (None): the accumulated per-player sets, i.e.
        //   the distinct affiliations/regions of every validator who has
        //   contributed a milestone for this player (including this one).
        let diversity_config = Self::get_diversity_config(env.clone());
        let min_region_quorum = Self::get_min_region_quorum(env.clone());
        let mut advance_allowed = true;
        match &attester_diversity {
            Some(div) => {
                if let Some(config) = diversity_config {
                    if next_index >= config.starting_milestone_index
                        && div.affiliations.len() < config.required_distinct_affiliations
                    {
                        advance_allowed = false;
                    }
                }

                // Region-quorum gate: only Level-2/3 advances (milestone >= 2)
                // are gated. A configured quorum of 0 (the default) disables the
                // check entirely; a quorum of 1 is trivially satisfied by any
                // attesting set.
                if min_region_quorum > 1
                    && next_index >= REGION_QUORUM_GATED_LEVEL_START_MILESTONE
                    && div.regions.len() < min_region_quorum
                {
                    advance_allowed = false;
                }
            }
            None => {
                if let Some(config) = diversity_config {
                    if next_index >= config.starting_milestone_index
                        && player_affiliations.len() < config.required_distinct_affiliations
                    {
                        advance_allowed = false;
                    }
                }

                // Region-quorum gate: only Level-2/3 advances (milestone >= 2)
                // are gated. A configured quorum of 0 (the default) disables the
                // check entirely; a quorum of 1 is trivially satisfied by any
                // milestone.
                if min_region_quorum > 1
                    && next_index >= REGION_QUORUM_GATED_LEVEL_START_MILESTONE
                    && player_regions.len() < min_region_quorum
                {
                    advance_allowed = false;
                }
            }
        }

        if advance_allowed {
            if let Some(progress_addr) = env
                .storage()
                .instance()
                .get::<DataKey, Address>(&DataKey::ProgressContract)
            {
                let progress_client = progress_contract::Client::new(env, &progress_addr);
                match progress_client.try_advance_level(validator_wallet, &player_id, &next_index) {
                    Ok(_) => {}
                    Err(Ok(progress_contract::ProgressError::AlreadyAtMaxLevel)) => {
                        events::level_advancement_skipped(
                            env,
                            player_id,
                            &soroban_sdk::String::from_str(env, "AlreadyAtMaxLevel"),
                        );
                    }
                    Err(e) => {
                        let code = match &e {
                            Ok(pe) => *pe as u32,
                            Err(_) => 0u32,
                        };
                        events::progress_call_failed(env, player_id, code);
                        return Err(VerificationError::ProgressCallFailed);
                    }
                }
            } else {
                if !env.storage().instance().has(&DataKey::ProgressContract) {
                    events::progress_contract_not_set(env, player_id);
                }
            }
        } else {
            // The anti-collusion gate blocked the advance (the milestone is
            // still recorded). Emit a diagnostic so indexers can distinguish a
            // diversity-gate rejection from a missing-wiring warning.
            events::level_advancement_skipped(
                env,
                player_id,
                &soroban_sdk::String::from_str(env, "DiversityGateNotMet"),
            );
            if !env.storage().instance().has(&DataKey::ProgressContract) {
                events::progress_contract_not_set(env, player_id);
            }
        }

        Ok(next_index)
    }

    /// Canonical attestation message bytes for ed25519 signing/verification.
    fn attestation_message(env: &Env, attestation: &MilestoneAttestation) -> Bytes {
        let mut message = Bytes::new(env);
        message.extend_from_slice(ATTESTATION_DOMAIN.as_bytes());
        message.append(&attestation.contract_id.clone().to_xdr(env));
        message.append(&Bytes::from_slice(env, &attestation.network_id.to_array()));
        message.append(&attestation.validator_wallet.clone().to_xdr(env));
        message.extend_from_slice(&attestation.player_id.to_be_bytes());
        message.append(&attestation.description.clone().to_xdr(env));
        message.append(&attestation.evidence_hash.clone().to_xdr(env));
        message.extend_from_slice(&attestation.nonce.to_be_bytes());
        message
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
        let id = env.register(VerificationContract, ());
        let client = VerificationContractClient::new(&env, &id);
        (env, client)
    }

    /// Minimal stub that mimics the registration contract's `get_player`.
    /// Returns a profile whose `wallet` is the one stored at init time,
    /// keyed by `player_id`.
    #[contract]
    struct RegStub;

    #[contracttype]
    enum StubKey {
        Owner,
    }

    #[contractimpl]
    impl RegStub {
        pub fn initialize(env: Env, owner: Address) {
            env.storage().persistent().set(&StubKey::Owner, &owner);
        }

        pub fn get_player(env: Env, player_id: u64) -> RegPlayerProfile {
            let wallet: Address = env.storage().persistent().get(&StubKey::Owner).unwrap();
            RegPlayerProfile {
                player_id,
                wallet,
                vitals: RegPlayerVitals {
                    age: 20,
                    position: String::from_str(&env, "Forward"),
                    region: String::from_str(&env, "Europe"),
                    nationality: String::from_str(&env, "ES"),
                },
                ipfs_hashes: Vec::new(&env),
                level: ProgressLevel::Unverified,
                registered_at: 0,
                updated_at: 0,
            }
        }
    }

    /// Deploy a `RegStub` and wire it as the registration contract on the
    /// verification client.  `owner` is the wallet address that `RegStub`
    /// will return as the profile owner for every `get_player` lookup.
    fn setup_with_registration(
        env: &Env,
        client: &VerificationContractClient<'static>,
        owner: &Address,
    ) {
        let reg_id = env.register(RegStub, ());
        let reg_client = RegStubClient::new(env, &reg_id);
        reg_client.initialize(owner);
        client.set_registration_contract(&reg_id);
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

    /// Build a distinct, charset-valid CIDv1 (lowercase base32 a–z, 2–7) for
    /// `seed`. The fixed 57-char prefix is a known-valid CIDv1 body; the last
    /// two characters base32-encode the seed so every seed maps to a unique
    /// hash (59 + 2 * distinct values), useful for tests that approve many
    /// milestones through the normal path.
    fn valid_cid_v1_for_seed(env: &Env, seed: u64) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
        let c1 = ALPHABET[((seed / 32) % 32) as usize] as char;
        let c2 = ALPHABET[(seed % 32) as usize] as char;
        let s = format!(
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbz{}{}",
            c1, c2
        );
        String::from_str(env, &s)
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
        client.register_validator(&validator, &String::from_str(&env, "Academy Director"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Use distinct players and evidence CIDs so the history exceeds the
        // 50-entry page cap through the normal approval path. Each CID is a
        // distinct, charset-valid CIDv1 (lowercase base32 a–z, 2–7).
        for player_id in 1u64..=51 {
            let evidence = valid_cid_v1_for_seed(&env, player_id);
            client.approve_milestone(
                &validator,
                &player_id,
                &String::from_str(&env, "approved"),
                &evidence,
                &None,
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
        client.register_validator(&validator, &String::from_str(&env, "Academy Director"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "approved"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "second"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );

        let page = client.get_milestones_by_validator_page(&validator, &0, &5);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().player_id, 1u64);
        assert_eq!(
            page.get(0).unwrap().description,
            String::from_str(&env, "approved")
        );
        assert_eq!(page.get(1).unwrap().player_id, 2u64);
        assert_eq!(
            page.get(1).unwrap().evidence_hash,
            String::from_str(&env, VALID_CID_V0_2)
        );
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
        client.register_validator(&validator, &String::from_str(&env, "Senior Coach"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

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
            &None,
        );
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );
        client.approve_milestone(
            &validator,
            &3u64,
            &String::from_str(&env, "m3"),
            &String::from_str(&env, VALID_CID_V0_3),
            &None,
        );

        let players = client.get_validator_players(&validator);
        assert_eq!(players.len(), 3);
        assert!(players.contains(1u64));
        assert!(players.contains(2u64));
        assert!(players.contains(3u64));
    }

    /// Approving a second milestone for the same player must NOT add a duplicate
    /// player_id to the ValidatorPlayers index.
    #[test]
    fn test_get_validator_players_no_duplicates() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Senior Coach"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Approve two milestones for the same player
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V1),
            &None,
        );

        // player 1 must appear exactly once
        let players = client.get_validator_players(&validator);
        assert_eq!(players.len(), 1);
        assert!(players.contains(1u64));
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
        client.register_validator(&v1, &String::from_str(&env, "Pro Coach AA"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&v2, &String::from_str(&env, "Pro Coach BB"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &v1,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &v1,
            &2u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );
        client.approve_milestone(
            &v2,
            &3u64,
            &String::from_str(&env, "m3"),
            &String::from_str(&env, VALID_CID_V0_3),
            &None,
        );

        let v1_players = client.get_validator_players(&v1);
        assert_eq!(v1_players.len(), 2);
        assert!(v1_players.contains(1u64));
        assert!(v1_players.contains(2u64));
        assert!(!v1_players.contains(3u64));

        let v2_players = client.get_validator_players(&v2);
        assert_eq!(v2_players.len(), 1);
        assert!(v2_players.contains(3u64));
    }

    #[test]
    fn test_validator_milestone_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

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
                &None,
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
        client.register_validator(&v1, &String::from_str(&env, "UEFA-B-CoachA"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&v2, &String::from_str(&env, "UEFA-B-CoachB"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &v1,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        assert_eq!(client.get_total_milestone_count(), 1);

        let v0_2 = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqC");
        let v0_3 = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqD");
        client.approve_milestone(&v1, &2u64, &String::from_str(&env, "m2"), &v0_2, &None);
        client.approve_milestone(&v2, &3u64, &String::from_str(&env, "m3"), &v0_3, &None);
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        assert!(client.is_active_validator(&validator));

        // No progress contract set — approve_milestone still records the milestone
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Scored 5 goals in Local Cup"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let idx1 = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        let idx2 = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, VALID_CID_V1),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        let reason: Option<String> = None;
        client.revoke_validator(&validator, &RevocationSeverity::Routine, &reason);

        assert!(!client.is_active_validator(&validator));
    }

    #[test]
    fn test_revoke_validator_with_reason() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        let reason = Some(String::from_str(&env, "Misconduct and protocol violation"));
        client.revoke_validator(&validator, &RevocationSeverity::Routine, &reason);

        assert!(!client.is_active_validator(&validator));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn test_revoke_validator_reason_too_long() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        // 129-byte string
        let long_reason = "x".repeat(129);
        let reason = Some(String::from_str(&env, &long_reason));
        client.revoke_validator(&validator, &RevocationSeverity::Routine, &reason);
    }

    #[test]
    #[should_panic]
    fn test_revoked_validator_cannot_approve() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        let reason: Option<String> = None;
        client.revoke_validator(&validator, &RevocationSeverity::Routine, &reason);

        // Should panic — validator is inactive
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
            &None,
        );
    }

    #[test]
    fn test_two_validators_approve_milestones_for_same_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator1 = Address::generate(&env);
        let validator2 = Address::generate(&env);
        client.register_validator(&validator1, &String::from_str(&env, "UEFA-B-CoachA"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&validator2, &String::from_str(&env, "UEFA-B-CoachB"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator1,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator2,
            &1u64,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, VALID_CID_V1),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.pause_contract();

        // Should panic — contract is paused
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Some milestone"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_approve_milestone_overflow() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

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
            &None,
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

        // set_progress_contract now emits both the legacy
        // progress_contract_updated event and the new wiring_updated event
        // (issue #1041) on every successful call.
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
                    addr.clone().into_val(&env)
                ),
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, crate::events::WIRING_UPDATED),
                        admin.clone(),
                        Symbol::new(&env, "progress_contract"),
                    )
                        .into_val(&env),
                    (addr, 1u32).into_val(&env)
                )
            ]
        );
    }

    #[test]
    fn test_update_progress_contract_succeeds() {
        // Regression test: verification's legacy first-call-only guard
        // (set_progress_contract → AlreadyConfigured on re-call, see
        // test_set_progress_contract_second_call_returns_already_configured)
        // must remain paired with a still-functional update_progress_contract
        // escape hatch for intentional re-wiring (issue #1041 keeps this path
        // deprecated-but-functional, not removed).
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let addr1 = Address::generate(&env);
        let addr2 = Address::generate(&env);
        client.set_progress_contract(&addr1);
        client.update_progress_contract(&addr2);

        let state = client.get_wiring_state();
        assert_eq!(state.progress_contract.address, Some(addr2));
        assert_eq!(
            state.progress_contract.epoch, 2,
            "update_progress_contract must still bump the epoch, same as any other wiring setter"
        );

        // The legacy guard itself must still be intact: a further
        // set_progress_contract call is still rejected, only
        // update_progress_contract may re-wire past the first call.
        let addr3 = Address::generate(&env);
        let result = client.try_set_progress_contract(&addr3);
        assert_eq!(result, Err(Ok(VerificationError::AlreadyConfigured)));
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
        assert_eq!(state.registration_contract.address, None);
        assert_eq!(state.registration_contract.epoch, 0);
        assert!(!state.is_fully_wired());
    }

    #[test]
    fn test_get_wiring_state_reflects_both_links() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let progress_addr = Address::generate(&env);
        let reg_addr = Address::generate(&env);
        client.set_progress_contract(&progress_addr);
        client.set_registration_contract(&reg_addr);

        let state = client.get_wiring_state();
        assert_eq!(state.progress_contract.address, Some(progress_addr));
        assert_eq!(state.progress_contract.epoch, 1);
        assert_eq!(state.registration_contract.address, Some(reg_addr));
        assert_eq!(state.registration_contract.epoch, 1);
        assert!(state.is_fully_wired());
    }

    #[test]
    fn test_set_registration_contract_second_call_returns_already_configured() {
        // registration_contract carries the same first-call-only legacy
        // guard as progress_contract (both predate issue #1041) — verify it
        // is untouched by the wiring-epoch rollout.
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let addr = Address::generate(&env);
        client.set_registration_contract(&addr);

        let result = client.try_set_registration_contract(&addr);
        assert_eq!(result, Err(Ok(VerificationError::AlreadyConfigured)));
    }

    #[test]
    fn test_update_registration_contract_bumps_epoch() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let addr1 = Address::generate(&env);
        let addr2 = Address::generate(&env);
        client.set_registration_contract(&addr1);
        client.update_registration_contract(&addr2);

        let state = client.get_wiring_state();
        assert_eq!(state.registration_contract.address, Some(addr2));
        assert_eq!(state.registration_contract.epoch, 2);
    }

    #[test]
    fn test_set_registration_contract_emits_wiring_updated_event() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let reg_addr = Address::generate(&env);
        client.set_registration_contract(&reg_addr);

        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    client.address.clone(),
                    (
                        Symbol::new(&env, crate::events::WIRING_UPDATED),
                        admin.clone(),
                        Symbol::new(&env, "registration_contract"),
                    )
                        .into_val(&env),
                    (reg_addr, 1u32).into_val(&env)
                )
            ]
        );
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let new_wasm_hash = env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::new(&env));
        client.upgrade(&new_wasm_hash);

        // Admin persisted — admin-gated call still works
        client.revoke_validator(&validator, &RevocationSeverity::Routine, &None);
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
        client.register_validator(&validator, &String::from_str(&env, &too_long), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
    }

    #[test]
    fn test_register_validator_credentials_256_bytes_succeeds() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        // Exactly 256 ASCII bytes — must be accepted
        let exactly_256 = "a".repeat(256);
        client.register_validator(&validator, &String::from_str(&env, &exactly_256), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

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
            client.register_validator(&v, &String::from_str(&env, "Credentials"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        }

        // The 101st registration must return ValidatorCapReached, not panic.
        let extra = Address::generate(&env);
        let result = client.try_register_validator(&extra, &String::from_str(&env, "Credentials"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
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

        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let reason: Option<String> = None;
        client.revoke_validator(&v2, &RevocationSeverity::Routine, &reason);

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

        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_eq!(client.get_active_validator_count(), 1);

        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_eq!(client.get_active_validator_count(), 2);

        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_eq!(client.get_active_validator_count(), 3);

        let reason: Option<String> = None;
        client.revoke_validator(&v2, &RevocationSeverity::Routine, &reason);
        assert_eq!(client.get_active_validator_count(), 2);

        client.revoke_validator(&v3, &RevocationSeverity::Routine, &reason);
        assert_eq!(client.get_active_validator_count(), 1);

        // Revoking an already-revoked validator should not change the count
        client.revoke_validator(&v3, &RevocationSeverity::Routine, &reason);
        assert_eq!(client.get_active_validator_count(), 1);
    }

    #[test]
    fn test_active_validator_count_matches_active_validator_statuses() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let assert_active_count_matches_statuses = || {
            let validators = client.get_validators();
            let mut active_by_status = 0u32;
            for wallet in validators.iter() {
                if client.get_validator_status(&wallet) == types::ValidatorStatus::Active {
                    active_by_status += 1;
                }
            }

            assert_eq!(client.get_active_validator_count(), active_by_status);
        };

        let v1 = Address::generate(&env);
        let v2 = Address::generate(&env);
        let v3 = Address::generate(&env);
        let reason: Option<String> = None;

        assert_active_count_matches_statuses();

        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_active_count_matches_statuses();

        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_active_count_matches_statuses();

        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_active_count_matches_statuses();

        client.revoke_validator(&v2, &RevocationSeverity::Routine, &reason);
        assert_active_count_matches_statuses();

        client.revoke_validator(&v3, &RevocationSeverity::Routine, &reason);
        assert_active_count_matches_statuses();

        client.restore_validator(&v2);
        assert_active_count_matches_statuses();

        client.revoke_validator(&v1, &RevocationSeverity::Routine, &reason);
        assert_active_count_matches_statuses();

        client.restore_validator(&v3);
        assert_active_count_matches_statuses();
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
        client.register_validator(&v1, &String::from_str(&env, "Credentials 1"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_eq!(client.get_validator_count(), 1);
        assert_eq!(client.get_validators().len(), 1); // get_validators() returns active only, which matches total

        client.register_validator(&v2, &String::from_str(&env, "Credentials 2"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_eq!(client.get_validator_count(), 2);
        assert_eq!(client.get_validators().len(), 2);

        client.register_validator(&v3, &String::from_str(&env, "Credentials 3"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert_eq!(client.get_validator_count(), 3);
        assert_eq!(client.get_validators().len(), 3);

        // Revoke some validators - total count should remain 3, active count decreases
        let reason: Option<String> = None;
        client.revoke_validator(&v2, &RevocationSeverity::Routine, &reason);
        assert_eq!(client.get_validator_count(), 3); // total still 3
        assert_eq!(client.get_active_validator_count(), 2); // active decreased to 2
        assert_eq!(client.get_validators().len(), 2); // get_validators() returns active only

        client.revoke_validator(&v3, &RevocationSeverity::Routine, &reason);
        assert_eq!(client.get_validator_count(), 3); // total still 3
        assert_eq!(client.get_active_validator_count(), 1); // active decreased to 1
        assert_eq!(client.get_validators().len(), 1); // get_validators() returns active only

        // Revoking an already-revoked validator should not change either count
        client.revoke_validator(&v3, &RevocationSeverity::Routine, &reason);
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        // 45 chars starting with Qm — one short of valid CIDv0
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygpq"),
            &None,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_cidv0_too_long_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        // 47 chars starting with Qm — one over valid CIDv0
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqBX"),
            &None,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_cidv0_invalid_base58_char_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        // 46 chars but contains '0' which is invalid in base58btc
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "Qm0K1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
            &None,
        );
    }

    #[test]
    fn test_cidv0_exactly_46_chars_accepted() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        // 58 chars starting with bafy — one short of valid CIDv1
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(
                &env,
                "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzd",
            ),
            &None,
        );
    }

    #[test]
    fn test_cidv1_exactly_59_chars_accepted() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, VALID_CID_V1),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "test"),
            &String::from_str(&env, "zdj7WbTaiJT1fgatdet7Sjxf4PJQgXkGfXPFgq5a2SdxYqYg"),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 1u64;
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 42u64;
        let description = String::from_str(&env, "Speed test passed 30 km/h");
        let evidence_hash = String::from_str(&env, VALID_CID_V0);

        let ledger_seq_at_approval = env.ledger().sequence();

        let idx =
            client.approve_milestone(&validator, &player_id, &description, &evidence_hash, &None);
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
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 7u64;
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Goal scored"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m2"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );

        assert_eq!(client.get_active_disputes_count(), 0);

        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
            &0u32,
        );
        assert_eq!(client.get_active_disputes_count(), 1);

        client.dispute_milestone(
            &player_wallet,
            &2u64,
            &1u32,
            &String::from_str(&env, "Also wrong"),
            &0u32,
        );
        assert_eq!(client.get_active_disputes_count(), 2);
    }

    /// Count is not affected by dispute_milestone on the same (player, index) —
    /// the duplicate is rejected before the counter increments.
    #[test]
    fn test_active_disputes_count_not_incremented_on_duplicate_dispute() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );

        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "First dispute"),
            &0u32,
        );
        assert_eq!(client.get_active_disputes_count(), 1);

        // Second dispute on the same (player, index) should fail
        let result = client.try_dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Second attempt"),
            &0u32,
        );
        assert!(result.is_err());
        // Count must remain 1
        assert_eq!(client.get_active_disputes_count(), 1);
    }

    #[test]
    fn test_resolve_dispute_marks_resolved_and_decrements_active_count() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
            &0u32,
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
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.dispute_milestone(
            &player_wallet,
            &2u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
            &0u32,
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
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &2u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );

        let reason = String::from_str(&env, "Wrong attribution");
        client.dispute_milestone(&player_wallet, &2u64, &1u32, &reason, &0u32);

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
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        client.approve_milestone(
            &validator,
            &3u64,
            &String::from_str(&env, "m1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.dispute_milestone(
            &player_wallet,
            &3u64,
            &1u32,
            &String::from_str(&env, "Wrong attribution"),
            &0u32,
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
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 1u64;
        let milestone_index: u32 = 1u32;

        // Approve a milestone so we have something to dispute
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );

        // Before dispute: must return false
        assert!(!client.has_dispute(&player_id, &milestone_index));

        // Submit dispute
        client.dispute_milestone(
            &player_wallet,
            &player_id,
            &milestone_index,
            &String::from_str(&env, "Milestone was not completed"),
            &0u32,
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
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Approve two milestones for player 1
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Milestone one"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Milestone two"),
            &String::from_str(&env, VALID_CID_V1),
            &None,
        );

        // Dispute only the first milestone
        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &1u32,
            &String::from_str(&env, "Disputed"),
            &0u32,
        );

        // The disputed milestone returns true
        assert!(client.has_dispute(&1u64, &1u32));
        // The undisputed milestone returns false
        assert!(!client.has_dispute(&1u64, &2u32));
        // A completely unknown player/index also returns false
        assert!(!client.has_dispute(&999u64, &1u32));
    }

    /// `has_dispute` is a thin boolean wrapper around `get_dispute`: it returns
    /// true exactly when `get_dispute` can load a dispute, and false exactly
    /// when `get_dispute` reports `MilestoneNotFound`.
    #[test]
    fn test_has_dispute_matches_get_dispute_ok_and_milestone_not_found() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let disputed_player_id = 1u64;
        let disputed_milestone_index = 1u32;
        let undisputed_milestone_index = 2u32;

        client.approve_milestone(
            &validator,
            &disputed_player_id,
            &String::from_str(&env, "Disputed milestone"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator,
            &disputed_player_id,
            &String::from_str(&env, "Never-disputed milestone"),
            &String::from_str(&env, VALID_CID_V1),
            &None,
        );

        client.dispute_milestone(
            &player_wallet,
            &disputed_player_id,
            &disputed_milestone_index,
            &String::from_str(&env, "Dispute reason"),
            &0u32,
        );

        let existing_dispute =
            client.try_get_dispute(&disputed_player_id, &disputed_milestone_index);
        assert!(existing_dispute.is_ok());
        assert!(client.has_dispute(&disputed_player_id, &disputed_milestone_index));

        let missing_dispute =
            client.try_get_dispute(&disputed_player_id, &undisputed_milestone_index);
        assert_eq!(
            missing_dispute,
            Err(Ok(VerificationError::MilestoneNotFound))
        );
        assert!(!client.has_dispute(&disputed_player_id, &undisputed_milestone_index));
    }

    // -------------------------------------------------------------------------
    // dispute_milestone wallet-authorization tests (issue #1014)
    // -------------------------------------------------------------------------

    /// An unrelated wallet cannot dispute another player's milestone.
    /// The registration contract stub returns `player_wallet` as the owner
    /// for every player_id, so `attacker_wallet` should be rejected.
    #[test]
    fn test_dispute_milestone_rejects_wrong_wallet() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let player_wallet = Address::generate(&env);
        let attacker_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 1;
        let milestone_index: u32 = 1;

        // Approve a milestone so there is something to dispute
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Goal scored"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );

        // Attacker (wrong wallet) tries to dispute — must fail
        let result = client.try_dispute_milestone(
            &attacker_wallet,
            &player_id,
            &milestone_index,
            &String::from_str(&env, "Fabricated reason"),
            &0u32,
        );
        assert_eq!(result, Err(Ok(VerificationError::Unauthorized)));
    }

    /// The actual player (correct wallet) can still dispute their own
    /// milestone — the authorization check must not break the legitimate
    /// happy path.
    #[test]
    fn test_dispute_milestone_accepts_correct_wallet() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let player_wallet = Address::generate(&env);
        client.initialize(&admin);
        setup_with_registration(&env, &client, &player_wallet);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 1;
        let milestone_index: u32 = 1;

        // Approve a milestone so there is something to dispute
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Goal scored"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );

        // Correct wallet disputes — must succeed
        let result = client.try_dispute_milestone(
            &player_wallet,
            &player_id,
            &milestone_index,
            &String::from_str(&env, "Legitimate concern"),
            &0u32,
        );
        assert!(result.is_ok());
        assert!(client.has_dispute(&player_id, &milestone_index));
    }

    ///   2. Attempt to register the same wallet again
    ///   3. Assert the second registration returns ValidatorAlreadyRegistered error
    ///   4. Verify the validator record in storage is unchanged
    ///   5. Verify the ValidatorVector length remains 1 (no duplicate added)
    ///
    /// **Validates: Duplicate registration check inregister_validator**
    #[test]
    fn test_register_validator_already_registered_wallet_fails() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let credentials = String::from_str(&env, "UEFA A License");

        // First registration succeeds
        client.register_validator(&validator, &credentials, &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        assert!(client.is_active_validator(&validator));

        // Verify validator is in the vector
        let validators = client.get_validators();
        assert_eq!(validators.len(), 1);
        assert_eq!(validators.get(0).unwrap(), validator);

        // Second registration with the same wallet should fail
        let result = client.try_register_validator(&validator, &String::from_str(&env, "Different credentials"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
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
        client.register_validator(&old_wallet, &credentials, &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Record a milestone to verify milestones get migrated
        client.approve_milestone(
            &old_wallet,
            &1u64,
            &String::from_str(&env, "Scored 5 goals"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
        client.register_validator(&wallet, &credentials, &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Record a milestone to verify milestone count remains intact
        client.approve_milestone(
            &wallet,
            &1u64,
            &String::from_str(&env, "Scored 5 goals"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
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
        client.register_validator(&wallet_cause, &String::from_str(&env, "Coach A License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&wallet_routine, &String::from_str(&env, "Coach B License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Approve milestones
        client.approve_milestone(
            &wallet_cause,
            &1u64,
            &String::from_str(&env, "M1"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &wallet_routine,
            &2u64,
            &String::from_str(&env, "M2"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );

        // Revoke with cause
        client.revoke_validator(
            &wallet_cause,
            &RevocationSeverity::ForCause,
            &Some(String::from_str(&env, "Misconduct")),
        );
        // Revoke for routine
        client.revoke_validator(
            &wallet_routine,
            &RevocationSeverity::Routine,
            &Some(String::from_str(&env, "Routine")),
        );

        // Check validator status
        assert_eq!(
            client.get_validator_status(&wallet_cause),
            types::ValidatorStatus::RevokedForCause
        );
        assert_eq!(
            client.get_validator_status(&wallet_routine),
            types::ValidatorStatus::Revoked
        );

        // Check milestone with status
        let milestone_cause = client.get_milestone_with_status(&1u64, &1u32);
        assert_eq!(
            milestone_cause.validator_status,
            types::ValidatorStatus::RevokedForCause
        );

        let milestone_routine = client.get_milestone_with_status(&2u64, &1u32);
        assert_eq!(
            milestone_routine.validator_status,
            types::ValidatorStatus::Revoked
        );

        // Restore validator and verify the flag is cleared
        client.restore_validator(&wallet_cause);
        assert_eq!(
            client.get_validator_status(&wallet_cause),
            types::ValidatorStatus::Active
        );
        let milestone_restored = client.get_milestone_with_status(&1u64, &1u32);
        assert_eq!(
            milestone_restored.validator_status,
            types::ValidatorStatus::Active
        );
    }

    // -------------------------------------------------------------------------
    // get_validator_statuses batch query tests (#850)
    // -------------------------------------------------------------------------

    /// Batch query returns one entry per input wallet, including NotRegistered
    /// for wallets that have never been registered.  A mixed batch of active,
    /// revoked, and never-registered wallets must all be reflected correctly.
    #[test]
    fn test_get_validator_statuses_mixed_batch() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let active_wallet = Address::generate(&env);
        let revoked_wallet = Address::generate(&env);
        let unregistered_wallet = Address::generate(&env);

        // Register both wallets as validators.
        client.register_validator(&active_wallet, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));
        client.register_validator(&revoked_wallet, &String::from_str(&env, "UEFA-A-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // Revoke one of them with the routine-revocation marker so the
        // status is plain `Revoked` (any reason other than "Routine" would
        // surface as `RevokedForCause`).
        let reason = Some(String::from_str(&env, "Routine"));
        client.revoke_validator(&revoked_wallet, &RevocationSeverity::Routine, &reason);

        // Batch-query all three wallets.
        let wallets = soroban_sdk::vec![
            &env,
            active_wallet.clone(),
            revoked_wallet.clone(),
            unregistered_wallet.clone(),
        ];
        let statuses = client.get_validator_statuses(&wallets);

        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses.get(0).unwrap(), types::ValidatorStatus::Active);
        assert_eq!(statuses.get(1).unwrap(), types::ValidatorStatus::Revoked);
        assert_eq!(
            statuses.get(2).unwrap(),
            types::ValidatorStatus::NotRegistered
        );
    }

    /// Batch is capped at 20 entries; wallets beyond the cap are silently
    /// ignored and the result length equals 20, not the input length.
    #[test]
    fn test_get_validator_statuses_batch_cap_at_20() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Build a Vec of 25 distinct wallets (none registered).
        let mut wallets = soroban_sdk::Vec::new(&env);
        for _ in 0..25 {
            wallets.push_back(Address::generate(&env));
        }

        let statuses = client.get_validator_statuses(&wallets);

        // Result must be capped at 20.
        assert_eq!(statuses.len(), 20);
        // All entries must be NotRegistered.
        for i in 0..20 {
            assert_eq!(
                statuses.get(i).unwrap(),
                types::ValidatorStatus::NotRegistered
            );
        }
    }

    /// An empty input returns an empty result without error.
    #[test]
    fn test_get_validator_statuses_empty_input() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let wallets: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&env);
        let statuses = client.get_validator_statuses(&wallets);
        assert_eq!(statuses.len(), 0);
    }

    // -------------------------------------------------------------------------
    // #860: get_milestones_since — mirrors progress.get_history_since semantics
    // -------------------------------------------------------------------------

    /// Returns only milestones with `approved_at >= since_timestamp`,
    /// matching the established `get_history_since` contract in the progress
    /// contract (issue #860).
    #[test]
    fn test_get_milestones_since_filters_by_approved_at() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let player_id: u64 = 1;

        // Milestone 1 at timestamp 100.
        env.ledger().with_mut(|l| l.timestamp = 100);
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Scored 3 goals"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );

        // Milestone 2 at timestamp 200.
        env.ledger().with_mut(|l| l.timestamp = 200);
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Top speed 32 km/h"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );

        // Milestone 3 at timestamp 300.
        env.ledger().with_mut(|l| l.timestamp = 300);
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "MVP in tournament"),
            &String::from_str(&env, VALID_CID_V0_3),
            &None,
        );

        // since_timestamp = 200 should return milestones 2 and 3 only.
        let result = client.get_milestones_since(&player_id, &200u64);
        assert_eq!(result.len(), 2);
        assert_eq!(result.get(0).unwrap().approved_at, 200);
        assert_eq!(result.get(1).unwrap().approved_at, 300);

        // since_timestamp = 0 returns all three milestones.
        let all = client.get_milestones_since(&player_id, &0u64);
        assert_eq!(all.len(), 3);

        // since_timestamp = 301 returns none.
        let none = client.get_milestones_since(&player_id, &301u64);
        assert_eq!(none.len(), 0);

        // since_timestamp = 300 returns only the last milestone (boundary is inclusive).
        let boundary = client.get_milestones_since(&player_id, &300u64);
        assert_eq!(boundary.len(), 1);
        assert_eq!(boundary.get(0).unwrap().approved_at, 300);
    }

    /// Player with no milestones returns an empty Vec.
    #[test]
    fn test_get_milestones_since_empty_for_unknown_player() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let result = client.get_milestones_since(&999u64, &0u64);
        assert_eq!(result.len(), 0);
    }

    // -------------------------------------------------------------------------
    // #865: get_validator_activity_report aggregate query
    // -------------------------------------------------------------------------

    /// The aggregate report's fields exactly match what the four individual
    /// queries return for the same validator.
    #[test]
    fn test_get_validator_activity_report_matches_individual_queries() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let player_id: u64 = 1;
        let player_id_2: u64 = 2;

        // Register the validator with specializations
        let mut specs = Vec::new(&env);
        specs.push_back(String::from_str(&env, "physical-stats"));
        client.register_validator(&validator, &String::from_str(&env, "UEFA B License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &specs);

        // Approve milestones for two distinct players
        client.approve_milestone(
            &validator,
            &player_id,
            &String::from_str(&env, "Scored 5 goals"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        client.approve_milestone(
            &validator,
            &player_id_2,
            &String::from_str(&env, "Speed test passed"),
            &String::from_str(&env, VALID_CID_V0_2),
            &None,
        );

        // Get individual query results
        let individual_validator = client.get_validator(&validator);
        let individual_status = client.get_validator_status(&validator);
        let individual_count = client.get_validator_milestone_count(&validator);
        let individual_players = client.get_validator_players(&validator);

        // Get aggregate report
        let report = client.get_validator_activity_report(&validator);

        // Verify the aggregate matches every individual query exactly
        assert_eq!(report.wallet, validator, "wallet mismatch");
        assert_eq!(
            report.credentials, individual_validator.credentials,
            "credentials mismatch"
        );
        assert_eq!(
            report.registered_at, individual_validator.registered_at,
            "registered_at mismatch"
        );
        assert_eq!(
            report.active, individual_validator.active,
            "active mismatch"
        );
        assert_eq!(report.status, individual_status, "status mismatch");
        assert_eq!(
            report.milestone_count, individual_count,
            "milestone_count mismatch"
        );
        assert_eq!(
            report.distinct_player_count,
            individual_players.len(),
            "distinct_player_count mismatch"
        );
        assert_eq!(
            report.distinct_players, individual_players,
            "distinct_players mismatch"
        );

        // Sanity-check expected values
        assert_eq!(report.milestone_count, 2);
        assert_eq!(report.distinct_player_count, 2);
        assert_eq!(report.status, types::ValidatorStatus::Active);
    }

    /// Report for an unregistered wallet returns ValidatorNotFound.
    #[test]
    fn test_get_validator_activity_report_unregistered_returns_not_found() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let unknown = Address::generate(&env);
        let result = client.try_get_validator_activity_report(&unknown);
        assert_eq!(result, Err(Ok(VerificationError::ValidatorNotFound)));
    }

    /// Report for a validator with no milestones has zero counts.
    #[test]
    fn test_get_validator_activity_report_zero_milestones() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "KYC Certificate"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        let report = client.get_validator_activity_report(&validator);
        assert_eq!(report.milestone_count, 0);
        assert_eq!(report.distinct_player_count, 0);
        assert_eq!(report.distinct_players.len(), 0);
        assert_eq!(report.status, types::ValidatorStatus::Active);
    }

    // -------------------------------------------------------------------------
    // Specialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_specializations_stored_on_validator() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let mut specs = Vec::new(&env);
        specs.push_back(String::from_str(&env, "physical-stats"));
        client.register_validator(&validator, &String::from_str(&env, "Coach License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &specs);

        let v = client.get_validator(&validator);
        assert_eq!(v.specializations, specs);
    }

    #[test]
    fn test_min_region_quorum_default_is_zero() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        assert_eq!(client.get_min_region_quorum(), 0);
    }

    #[test]
    fn test_set_and_get_min_region_quorum() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);
        client.set_min_region_quorum(&2u32);
        assert_eq!(client.get_min_region_quorum(), 2);
    }

    /// A validator without the requested category cannot approve a tagged
    /// milestone (`SpecializationMismatch`).
    #[test]
    fn test_specialization_mismatch_rejected() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let mut specs = Vec::new(&env);
        specs.push_back(String::from_str(&env, "physical-stats"));
        client.register_validator(&validator, &String::from_str(&env, "Coach A License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &specs);

        // Tagged with a category the validator does not have.
        let result = client.try_approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
            &Some(String::from_str(&env, "identity-kyc")),
        );
        assert_eq!(result, Err(Ok(VerificationError::SpecializationMismatch)));
    }

    /// A validator with the matching category can approve a tagged milestone.
    #[test]
    fn test_matching_specialization_allows_advance() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        let mut specs = Vec::new(&env);
        specs.push_back(String::from_str(&env, "physical-stats"));
        client.register_validator(&validator, &String::from_str(&env, "Coach A License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &specs);

        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Identity verified"),
            &String::from_str(&env, VALID_CID_V0),
            &Some(String::from_str(&env, "physical-stats")),
        );
        assert_eq!(idx, 1);
    }

    /// When no milestone category is supplied, the specialization check is
    /// skipped entirely — a general-purpose validator can approve.
    #[test]
    fn test_no_category_skips_specialization_check() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let validator = Address::generate(&env);
        client.register_validator(&validator, &String::from_str(&env, "Coach A License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &Vec::new(&env));

        // No category → no specialization check.
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Identity"),
            &String::from_str(&env, VALID_CID_V0),
            &None,
        );
        assert_eq!(idx, 1);
    }
}
