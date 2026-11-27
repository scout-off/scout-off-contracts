// Events not yet migrated to #[contractevent] keep the legacy
// `env.events().publish` call so their wire format (topics and positional
// data) stays unchanged for indexers.
#![allow(deprecated)]
use soroban_sdk::{Address, Env, String, Symbol};

pub const MILESTONE_APPROVED: &str = "milestone_approved";
pub const VALIDATOR_REGISTERED: &str = "validator_registered";
pub const VALIDATOR_REVOKED: &str = "validator_revoked";
pub const VALIDATOR_REVOKED_FOR_CAUSE: &str = "validator_revoked_for_cause";
pub const VALIDATOR_RESTORED: &str = "validator_restored";
pub const VALIDATOR_TRANSFERRED: &str = "validator_transferred";
pub const CONTRACT_PAUSED: &str = "contract_paused";
pub const CONTRACT_UNPAUSED: &str = "contract_unpaused";
pub const APPROVE_MILESTONE_PAUSED: &str = "approve_milestone_paused";
pub const APPROVE_MILESTONE_UNPAUSED: &str = "approve_milestone_unpaused";
pub const CONTRACT_INITIALIZED: &str = "contract_initialized";
pub const PROGRESS_CONTRACT_UPDATED: &str = "progress_contract_updated";
pub const DISPUTE_RESOLVED: &str = "dispute_resolved";
pub const ADMIN_TRANSFER_PROPOSED: &str = "admin_transfer_proposed";
pub const ADMIN_TRANSFERRED: &str = "admin_transferred";
pub const ATTESTATION_RECORDED: &str = "attestation_recorded";
pub const ATTESTATION_WINDOW_EXPIRED: &str = "attestation_window_expired";
pub const VALIDATOR_PENDING_VOTES_INVALIDATED: &str = "validator_votes_invalidated";
pub const WIRING_UPDATED: &str = "wiring_updated";
pub const DISPUTE_VOTE_CAST: &str = "dispute_vote_cast";
pub const DISPUTE_TALLIED: &str = "dispute_tallied";
pub const PLAYER_PURGE_STARTED: &str = "player_purge_started";
pub const PLAYER_DATA_PURGED: &str = "player_data_purged";
pub const PLAYER_PURGE_COMPLETED: &str = "player_purge_completed";

/// topics: (event_name, old_admin)  data: new_admin
pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, ADMIN_TRANSFER_PROPOSED), old_admin.clone()),
        new_admin.clone(),
    );
}

/// Emitted by `approve_milestone` and `attest_milestone` (threshold commit).
#[contractevent]
pub struct MilestoneApproved {
    #[topic]
    pub validator: Address,
    pub player_id: u64,
    pub milestone_index: u32,
    pub description: String,
    pub evidence_hash: String,
}

/// Emitted by `register_validator`.
#[contractevent]
pub struct ValidatorRegistered {
    #[topic]
    pub wallet: Address,
    pub credentials: String,
}

/// Emitted by `revoke_validator` (routine).
#[contractevent]
pub struct ValidatorRevoked {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
    pub reason: String,
}

/// Emitted by `revoke_validator` (for-cause).
#[contractevent]
pub struct ValidatorRevokedForCause {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
    pub reason: String,
}

/// Emitted by `restore_validator`.
#[contractevent]
pub struct ValidatorRestored {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
}

/// Emitted by `set_validator_specializations`; `new` replaces the validator's
/// tag list and `old` is kept for audit history. Data is published as the
/// tuple `(wallet, old, new)` documented in docs/EVENT_REFERENCE.md.
#[contractevent(topics = ["validator_specialization_updated"], data_format = "vec")]
pub struct ValidatorSpecializationUpdated {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
    pub old: soroban_sdk::Vec<String>,
    pub new: soroban_sdk::Vec<String>,
}

/// Emitted by `transfer_validator`.
#[contractevent]
pub struct ValidatorTransferred {
    #[topic]
    pub admin: Address,
    pub old_wallet: Address,
    pub new_wallet: Address,
}

/// Emitted by `pause_contract`.
#[contractevent]
pub struct ContractPaused {
    #[topic]
    pub admin: Address,
}

/// Emitted by `unpause_contract`.
#[contractevent]
pub struct ContractUnpaused {
    #[topic]
    pub admin: Address,
}

/// Emitted by `pause_approve_milestone`.
#[contractevent]
pub struct ApproveMilestonePaused {
    #[topic]
    pub admin: Address,
}

/// Emitted by `unpause_approve_milestone`.
#[contractevent]
pub struct ApproveMilestoneUnpaused {
    #[topic]
    pub admin: Address,
}

/// Emitted by `set_progress_contract`.
#[contractevent]
pub struct ProgressContractUpdated {
    #[topic]
    pub admin: Address,
    pub progress_contract: Address,
}

/// Emitted by every `set_*_contract` wiring call.
#[contractevent]
pub struct WiringUpdated {
    #[topic]
    pub admin: Address,
    #[topic]
    pub link: soroban_sdk::Symbol,
    pub new_address: Address,
    pub new_epoch: u32,
}

/// Emitted by `dispute_milestone`.
#[contractevent]
pub struct MilestoneDisputed {
    #[topic]
    pub player_wallet: Address,
    pub player_id: u64,
    pub milestone_index: u32,
    pub reason: String,
}

/// Emitted by `resolve_dispute`.
#[contractevent]
pub struct DisputeResolved {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
    pub milestone_index: u32,
    pub upheld: bool,
}

/// Emitted when level advancement is skipped (player already at max).
#[contractevent]
pub struct LevelAdvancementSkipped {
    #[topic]
    pub player_id: u64,
    pub reason: String,
}

/// Emitted when progress contract address is not set.
#[contractevent]
pub struct ProgressContractNotSet {
    #[topic]
    pub player_id: u64,
}

/// Emitted on every accepted `attest_milestone` vote.
#[contractevent]
pub struct AttestationRecorded {
    #[topic]
    pub validator: Address,
    pub player_id: u64,
    pub evidence_hash: String,
    pub vote_count: u32,
    pub threshold: u32,
    pub description_hash: BytesN<32>,
}

/// Emitted when a sub-threshold claim's voting window expires.
#[contractevent]
pub struct AttestationWindowExpired {
    #[topic]
    pub player_id: u64,
    pub evidence_hash: String,
    pub new_round: u32,
}

/// Emitted when a revoked validator's pending votes are invalidated.
#[contractevent(topics = ["validator_votes_invalidated"])]
pub struct ValidatorPendingVotesInvalidated {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
    pub invalidated_count: u32,
}

/// Emitted when a revocation leaves an open k-of-n claim with a snapshotted
/// threshold above the active validator count, so the round is aborted and
/// its votes discarded (issue #1613). The evidence can then be re-attested.
#[contractevent]
pub struct AttestationRoundAborted {
    #[topic]
    pub player_id: u64,
    pub evidence_hash: String,
    pub round: u32,
    pub threshold: u32,
    pub active_count: u32,
}

/// Emitted when progress cross-contract call fails (diagnostic stream only).
#[contractevent]
pub struct ProgressCallFailed {
    #[topic]
    pub player_id: u64,
    pub error_code: u32,
}

/// Emitted by `restore_validator_record`.
#[contractevent]
pub struct ValidatorRecordRestored {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
}

/// Emitted by `restore_milestone_record`.
#[contractevent]
pub struct MilestoneRecordRestored {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
    pub index: u32,
}

/// Emitted for each milestone flagged during a for-cause revocation cascade.
#[contractevent]
pub struct MilestoneFlaggedForRereview {
    #[topic]
    pub validator: Address,
    pub player_id: u64,
    pub milestone_index: u32,
}

/// Emitted when a re-review flag is cleared.
#[contractevent]
pub struct MilestoneFlagCleared {
    #[topic]
    pub reviewer: Address,
    pub player_id: u64,
    pub milestone_index: u32,
}

/// Emitted when a for-cause cascade sweep completes.
#[contractevent]
pub struct RevocationCascadeComplete {
    #[topic]
    pub validator: Address,
    pub total_flagged: u32,
}

/// Emitted when a cascade sweep reaches per-call limit and a cursor is stored.
#[contractevent]
pub struct RevocationCascadeContinued {
    #[topic]
    pub validator: Address,
    pub next_cursor: u32,
    pub flagged_this_call: u32,
}

/// Emitted by `propose_admin`.
#[contractevent]
pub struct AdminTransferProposed {
    #[topic]
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Emitted by `accept_admin`.
#[contractevent]
pub struct AdminTransferred {
    #[topic]
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Emitted when a validator casts a vote on a jury-required dispute.
#[contractevent]
pub struct DisputeVoteCast {
    #[topic]
    pub validator: Address,
    pub player_id: u64,
    pub milestone_index: u32,
    pub for_upheld: bool,
}

/// Emitted when a jury-required dispute is tallied and resolved.
#[contractevent]
pub struct DisputeTallied {
    #[topic]
    pub player_id: u64,
    pub milestone_index: u32,
    pub upheld: bool,
    pub votes_for: u32,
    pub votes_against: u32,
}

/// Emitted by `purge_player_data` (GDPR erasure, issue #1373).
#[contractevent]
pub struct PlayerDataPurged {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
    pub entries_removed: u32,
    pub more: bool,
}

// ── Emit helpers ─────────────────────────────────────────────────────────────

pub fn contract_initialized(env: &Env, admin: &Address) {
    ContractInitialized {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn milestone_approved(
    env: &Env,
    player_id: u64,
    validator: &Address,
    milestone_index: u32,
    description: &String,
    evidence_hash: &String,
) {
    MilestoneApproved {
        validator: validator.clone(),
        player_id,
        milestone_index,
        description: description.clone(),
        evidence_hash: evidence_hash.clone(),
    }
    .publish(env);
}

pub fn validator_registered(env: &Env, wallet: &Address, credentials: &String) {
    ValidatorRegistered {
        wallet: wallet.clone(),
        credentials: credentials.clone(),
    }
    .publish(env);
}

/// topics: (event_name, wallet)  data: (public_key, rotated_from)
pub fn attestation_key_registered(
    env: &Env,
    wallet: &Address,
    public_key: &soroban_sdk::BytesN<32>,
    rotated_from: &Option<soroban_sdk::BytesN<32>>,
) {
    env.events().publish(
        (
            Symbol::new(env, "attestation_key_registered"),
            wallet.clone(),
        ),
        (public_key.clone(), rotated_from.clone()),
    );
}

/// topics: (event_name, admin)  data: (wallet, reason)
pub fn validator_revoked(env: &Env, admin: &Address, wallet: &Address, reason: &String) {
    ValidatorRevoked {
        admin: admin.clone(),
        wallet: wallet.clone(),
        reason: reason.clone(),
    }
    .publish(env);
}

/// Emitted by `batch_revoke_validators` (#1605).
/// topics: (event_name, admin)  data: (count, for_cause)
#[contractevent(topics = ["batch_revoked"], data_format = "vec")]
pub struct BatchRevoked {
    #[topic]
    pub admin: Address,
    pub count: u32,
    pub for_cause: bool,
}

pub fn batch_revoked(env: &Env, admin: &Address, count: u32, for_cause: bool) {
    BatchRevoked {
        admin: admin.clone(),
        count,
        for_cause,
    }
    .publish(env);
}

pub fn validator_revoked_for_cause(env: &Env, admin: &Address, wallet: &Address, reason: &String) {
    ValidatorRevokedForCause {
        admin: admin.clone(),
        wallet: wallet.clone(),
        reason: reason.clone(),
    }
    .publish(env);
}

pub fn validator_restored(env: &Env, admin: &Address, wallet: &Address) {
    ValidatorRestored {
        admin: admin.clone(),
        wallet: wallet.clone(),
    }
    .publish(env);
}

pub fn validator_transferred(
    env: &Env,
    admin: &Address,
    old_wallet: &Address,
    new_wallet: &Address,
) {
    ValidatorTransferred {
        admin: admin.clone(),
        old_wallet: old_wallet.clone(),
        new_wallet: new_wallet.clone(),
    }
    .publish(env);
}

pub fn validator_specialization_updated(
    env: &Env,
    admin: &Address,
    wallet: &Address,
    old: &soroban_sdk::Vec<String>,
    new: &soroban_sdk::Vec<String>,
) {
    ValidatorSpecializationUpdated {
        admin: admin.clone(),
        wallet: wallet.clone(),
        old: old.clone(),
        new: new.clone(),
    }
    .publish(env);
}

pub fn contract_paused(env: &Env, admin: &Address) {
    ContractPaused {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn contract_unpaused(env: &Env, admin: &Address) {
    ContractUnpaused {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn approve_milestone_paused(env: &Env, admin: &Address) {
    ApproveMilestonePaused {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn approve_milestone_unpaused(env: &Env, admin: &Address) {
    ApproveMilestoneUnpaused {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn progress_contract_updated(env: &Env, admin: &Address, progress_contract: &Address) {
    ProgressContractUpdated {
        admin: admin.clone(),
        progress_contract: progress_contract.clone(),
    }
    .publish(env);
}

pub fn wiring_updated(
    env: &Env,
    admin: &Address,
    link: &str,
    new_address: &Address,
    new_epoch: u32,
) {
    WiringUpdated {
        admin: admin.clone(),
        link: soroban_sdk::Symbol::new(env, link),
        new_address: new_address.clone(),
        new_epoch,
    }
    .publish(env);
}

pub fn milestone_disputed(
    env: &Env,
    player_wallet: &Address,
    player_id: u64,
    milestone_index: u32,
    reason: &String,
) {
    MilestoneDisputed {
        player_wallet: player_wallet.clone(),
        player_id,
        milestone_index,
        reason: reason.clone(),
    }
    .publish(env);
}

pub fn dispute_resolved(
    env: &Env,
    admin: &Address,
    player_id: u64,
    milestone_index: u32,
    upheld: bool,
) {
    DisputeResolved {
        admin: admin.clone(),
        player_id,
        milestone_index,
        upheld,
    }
    .publish(env);
}

pub fn level_advancement_skipped(env: &Env, player_id: u64, reason: &String) {
    LevelAdvancementSkipped {
        player_id,
        reason: reason.clone(),
    }
    .publish(env);
}

/// Emitted when diversity gating blocks level advancement (the milestone is
/// committed but the player's level has not advanced because they need more
/// distinct validator affiliations).
///
/// topics: (event_name, player_id)  data: (milestone_index, distinct_affiliations, required)
pub fn level_advancement_deferred(
    env: &Env,
    player_id: u64,
    milestone_index: u32,
    distinct_affiliations: u32,
    required: u32,
) {
    env.events().publish(
        (Symbol::new(env, "level_advancement_deferred"), player_id),
        (milestone_index, distinct_affiliations, required),
    );
}

/// Emitted when level advancement is skipped because the progress contract
/// address has not been configured.  Common during testing without a full
/// deployment.  In production this indicates a missing wiring step and the
/// indexer should alert on it.  The milestone is still persisted.
pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    ProgressContractNotSet { player_id }.publish(env);
}

/// topics: (event_name, validator)  data: (player_id, evidence_hash, vote_count, threshold, description_hash)
pub fn attestation_recorded(
    env: &Env,
    validator: &Address,
    player_id: u64,
    evidence_hash: &String,
    vote_count: u32,
    threshold: u32,
    description_hash: &BytesN<32>,
) {
    AttestationRecorded {
        validator: validator.clone(),
        player_id,
        evidence_hash: evidence_hash.clone(),
        vote_count,
        threshold,
        description_hash: description_hash.clone(),
    }
    .publish(env);
}

pub fn attestation_window_expired(
    env: &Env,
    player_id: u64,
    evidence_hash: &String,
    new_round: u32,
) {
    AttestationWindowExpired {
        player_id,
        evidence_hash: evidence_hash.clone(),
        new_round,
    }
    .publish(env);
}

pub fn validator_pending_votes_invalidated(
    env: &Env,
    admin: &Address,
    wallet: &Address,
    invalidated_count: u32,
) {
    ValidatorPendingVotesInvalidated {
        admin: admin.clone(),
        wallet: wallet.clone(),
        invalidated_count,
    }
    .publish(env);
}

pub fn attestation_round_aborted(
    env: &Env,
    player_id: u64,
    evidence_hash: &String,
    round: u32,
    threshold: u32,
    active_count: u32,
) {
    AttestationRoundAborted {
        player_id,
        evidence_hash: evidence_hash.clone(),
        round,
        threshold,
        active_count,
    }
    .publish(env);
}

pub fn progress_call_failed(env: &Env, player_id: u64, error_code: u32) {
    ProgressCallFailed {
        player_id,
        error_code,
    }
    .publish(env);
}

pub fn validator_record_restored(env: &Env, admin: &Address, wallet: &Address) {
    ValidatorRecordRestored {
        admin: admin.clone(),
        wallet: wallet.clone(),
    }
    .publish(env);
}

pub fn milestone_record_restored(env: &Env, admin: &Address, player_id: u64, index: u32) {
    MilestoneRecordRestored {
        admin: admin.clone(),
        player_id,
        index,
    }
    .publish(env);
}

pub fn milestone_flagged_for_rereview(
    env: &Env,
    validator: &Address,
    player_id: u64,
    milestone_index: u32,
) {
    MilestoneFlaggedForRereview {
        validator: validator.clone(),
        player_id,
        milestone_index,
    }
    .publish(env);
}

pub fn milestone_flag_cleared(env: &Env, reviewer: &Address, player_id: u64, milestone_index: u32) {
    MilestoneFlagCleared {
        reviewer: reviewer.clone(),
        player_id,
        milestone_index,
    }
    .publish(env);
}

pub fn revocation_cascade_complete(env: &Env, validator: &Address, total_flagged: u32) {
    RevocationCascadeComplete {
        validator: validator.clone(),
        total_flagged,
    }
    .publish(env);
}

pub fn revocation_cascade_continued(
    env: &Env,
    validator: &Address,
    next_cursor: u32,
    flagged_this_call: u32,
) {
    RevocationCascadeContinued {
        validator: validator.clone(),
        next_cursor,
        flagged_this_call,
    }
    .publish(env);
}

pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    AdminTransferProposed {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
    }
    .publish(env);
}

pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    AdminTransferred {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
    }
    .publish(env);
}

pub fn dispute_vote_cast(
    env: &Env,
    player_id: u64,
    milestone_index: u32,
    validator: &Address,
    for_upheld: bool,
) {
    DisputeVoteCast {
        validator: validator.clone(),
        player_id,
        milestone_index,
        for_upheld,
    }
    .publish(env);
}

pub fn dispute_tallied(
    env: &Env,
    player_id: u64,
    milestone_index: u32,
    upheld: bool,
    votes_for: u32,
    votes_against: u32,
) {
    DisputeTallied {
        player_id,
        milestone_index,
        upheld,
        votes_for,
        votes_against,
    }
    .publish(env);
}

pub fn player_data_purged(
    env: &Env,
    admin: &Address,
    player_id: u64,
    entries_removed: u32,
    more: bool,
) {
    PlayerDataPurged {
        admin: admin.clone(),
        player_id,
        entries_removed,
        more,
    }
    .publish(env);
}

/// Emitted before `update_current_contract_wasm` — attributed to the old code version.
/// topics: (event_name, admin)  data: new_wasm_hash
pub fn contract_upgraded(env: &Env, admin: &Address, new_wasm_hash: &BytesN<32>) {
    env.events().publish(
        (Symbol::new(env, CONTRACT_UPGRADED), admin.clone()),
        new_wasm_hash.clone(),
    );
}

/// Emitted by `open_migration_window`. While the window is open the
/// `admin_seed_*` entrypoints are enabled, so indexers should alert on this
/// event in production (see docs/EVENT_REFERENCE.md and docs/RUNBOOK.md).
///
/// topics: (event_name, admin)  data: ()
pub fn migration_window_opened(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, MIGRATION_WINDOW_OPENED), admin.clone()),
        (),
    );
}

// ── Player erasure events (issue #1781) ──────────────────────────────────────

/// Emitted on the **first** page of `purge_player_data` for a player.
/// topics: (event_name, admin)  data: player_id
pub fn player_purge_started(env: &Env, admin: &Address, player_id: u64) {
    env.events().publish(
        (Symbol::new(env, PLAYER_PURGE_STARTED), admin.clone()),
        player_id,
    );
}

/// Emitted on **every** page (including start and completion pages).
/// topics: (event_name, admin)  data: (player_id, entries_removed, next_cursor, more)
pub fn player_data_purged(
    env: &Env,
    admin: &Address,
    player_id: u64,
    entries_removed: u32,
    next_cursor: u32,
    more: bool,
) {
    env.events().publish(
        (Symbol::new(env, PLAYER_DATA_PURGED), admin.clone()),
        (player_id, entries_removed, next_cursor, more),
    );
}

/// Emitted on the **last** page of `purge_player_data` when no data remains.
/// topics: (event_name, admin)  data: player_id
pub fn player_purge_completed(env: &Env, admin: &Address, player_id: u64) {
    env.events().publish(
        (Symbol::new(env, PLAYER_PURGE_COMPLETED), admin.clone()),
        player_id,
    );
}
