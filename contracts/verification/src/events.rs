use soroban_sdk::{contractevent, Address, Env, String};

// ── Typed contract events (issue #1370) ──────────────────────────────────────

/// Emitted by `initialize` / `__constructor`.
#[contractevent]
pub struct ContractInitialized {
    #[topic]
    pub admin: Address,
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
#[contractevent]
pub struct ValidatorPendingVotesInvalidated {
    #[topic]
    pub admin: Address,
    pub wallet: Address,
    pub invalidated_count: u32,
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
    .emit(env);
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
    .emit(env);
}

pub fn validator_registered(env: &Env, wallet: &Address, credentials: &String) {
    ValidatorRegistered {
        wallet: wallet.clone(),
        credentials: credentials.clone(),
    }
    .emit(env);
}

pub fn validator_revoked(env: &Env, admin: &Address, wallet: &Address, reason: &String) {
    ValidatorRevoked {
        admin: admin.clone(),
        wallet: wallet.clone(),
        reason: reason.clone(),
    }
    .emit(env);
}

pub fn validator_revoked_for_cause(env: &Env, admin: &Address, wallet: &Address, reason: &String) {
    ValidatorRevokedForCause {
        admin: admin.clone(),
        wallet: wallet.clone(),
        reason: reason.clone(),
    }
    .emit(env);
}

pub fn validator_restored(env: &Env, admin: &Address, wallet: &Address) {
    ValidatorRestored {
        admin: admin.clone(),
        wallet: wallet.clone(),
    }
    .emit(env);
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
    .emit(env);
}

pub fn contract_paused(env: &Env, admin: &Address) {
    ContractPaused {
        admin: admin.clone(),
    }
    .emit(env);
}

pub fn contract_unpaused(env: &Env, admin: &Address) {
    ContractUnpaused {
        admin: admin.clone(),
    }
    .emit(env);
}

pub fn approve_milestone_paused(env: &Env, admin: &Address) {
    ApproveMilestonePaused {
        admin: admin.clone(),
    }
    .emit(env);
}

pub fn approve_milestone_unpaused(env: &Env, admin: &Address) {
    ApproveMilestoneUnpaused {
        admin: admin.clone(),
    }
    .emit(env);
}

pub fn progress_contract_updated(env: &Env, admin: &Address, progress_contract: &Address) {
    ProgressContractUpdated {
        admin: admin.clone(),
        progress_contract: progress_contract.clone(),
    }
    .emit(env);
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
    .emit(env);
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
    .emit(env);
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
    .emit(env);
}

pub fn level_advancement_skipped(env: &Env, player_id: u64, reason: &String) {
    LevelAdvancementSkipped {
        player_id,
        reason: reason.clone(),
    }
    .emit(env);
}

pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    ProgressContractNotSet { player_id }.emit(env);
}

pub fn attestation_recorded(
    env: &Env,
    validator: &Address,
    player_id: u64,
    evidence_hash: &String,
    vote_count: u32,
    threshold: u32,
) {
    AttestationRecorded {
        validator: validator.clone(),
        player_id,
        evidence_hash: evidence_hash.clone(),
        vote_count,
        threshold,
    }
    .emit(env);
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
    .emit(env);
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
    .emit(env);
}

pub fn progress_call_failed(env: &Env, player_id: u64, error_code: u32) {
    ProgressCallFailed {
        player_id,
        error_code,
    }
    .emit(env);
}

pub fn validator_record_restored(env: &Env, admin: &Address, wallet: &Address) {
    ValidatorRecordRestored {
        admin: admin.clone(),
        wallet: wallet.clone(),
    }
    .emit(env);
}

pub fn milestone_record_restored(env: &Env, admin: &Address, player_id: u64, index: u32) {
    MilestoneRecordRestored {
        admin: admin.clone(),
        player_id,
        index,
    }
    .emit(env);
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
    .emit(env);
}

pub fn milestone_flag_cleared(
    env: &Env,
    reviewer: &Address,
    player_id: u64,
    milestone_index: u32,
) {
    MilestoneFlagCleared {
        reviewer: reviewer.clone(),
        player_id,
        milestone_index,
    }
    .emit(env);
}

pub fn revocation_cascade_complete(env: &Env, validator: &Address, total_flagged: u32) {
    RevocationCascadeComplete {
        validator: validator.clone(),
        total_flagged,
    }
    .emit(env);
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
    .emit(env);
}

pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    AdminTransferProposed {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
    }
    .emit(env);
}

pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    AdminTransferred {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
    }
    .emit(env);
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
    .emit(env);
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
    .emit(env);
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
    .emit(env);
}
