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
pub const VALIDATOR_VOTES_INVALIDATED: &str = "validator_votes_invalidated";
pub const WIRING_UPDATED: &str = "wiring_updated";
pub const DISPUTE_VOTE_CAST: &str = "dispute_vote_cast";
pub const DISPUTE_TALLIED: &str = "dispute_tallied";
pub const MILESTONE_DISPUTED: &str = "milestone_disputed";
pub const LEVEL_ADVANCEMENT_SKIPPED: &str = "level_advancement_skipped";
pub const PROGRESS_CONTRACT_NOT_SET: &str = "progress_contract_not_set";
pub const PROGRESS_CALL_FAILED: &str = "progress_call_failed";
pub const VALIDATOR_RECORD_RESTORED: &str = "validator_record_restored";
pub const MILESTONE_RECORD_RESTORED: &str = "milestone_record_restored";
pub const MILESTONE_FLAGGED: &str = "milestone_flagged";
pub const MILESTONE_FLAG_CLEARED: &str = "milestone_flag_cleared";
pub const REVOCATION_CASCADE_COMPLETE: &str = "revocation_cascade_complete";
pub const REVOCATION_CASCADE_CONTINUED: &str = "revocation_cascade_continued";

/// topics: (event_name, old_admin)  data: new_admin
pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, ADMIN_TRANSFER_PROPOSED), old_admin.clone()),
        new_admin.clone(),
    );
}

/// topics: (event_name, old_admin)  data: new_admin
pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, ADMIN_TRANSFERRED), old_admin.clone()),
        new_admin.clone(),
    );
}

/// topics: (event_name, validator)  data: (player_id, description, evidence_hash)
pub fn milestone_approved(
    env: &Env,
    player_id: u64,
    validator: &Address,
    milestone_index: u32,
    description: &String,
    evidence_hash: &String,
) {
    env.events().publish(
        (Symbol::new(env, "milestone_approved"), validator.clone()),
        (
            player_id,
            milestone_index,
            description.clone(),
            evidence_hash.clone(),
        ),
    );
}

/// topics: (event_name, wallet)  data: credentials
pub fn validator_registered(env: &Env, wallet: &Address, credentials: &String) {
    env.events().publish(
        (Symbol::new(env, "validator_registered"), wallet.clone()),
        credentials.clone(),
    );
}

/// topics: (event_name, admin)  data: (wallet, reason)
pub fn validator_revoked(env: &Env, admin: &Address, wallet: &Address, reason: &String) {
    env.events().publish(
        (Symbol::new(env, "validator_revoked"), admin.clone()),
        (wallet.clone(), reason.clone()),
    );
}

/// topics: (event_name, admin)  data: (wallet, reason)
pub fn validator_revoked_for_cause(env: &Env, admin: &Address, wallet: &Address, reason: &String) {
    env.events().publish(
        (
            Symbol::new(env, "validator_revoked_for_cause"),
            admin.clone(),
        ),
        (wallet.clone(), reason.clone()),
    );
}

/// topics: (event_name, admin)  data: wallet
pub fn validator_restored(env: &Env, admin: &Address, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, VALIDATOR_RESTORED), admin.clone()),
        wallet.clone(),
    );
}

/// topics: (event_name, admin)  data: (old_wallet, new_wallet)
pub fn validator_transferred(
    env: &Env,
    admin: &Address,
    old_wallet: &Address,
    new_wallet: &Address,
) {
    env.events().publish(
        (Symbol::new(env, VALIDATOR_TRANSFERRED), admin.clone()),
        (old_wallet.clone(), new_wallet.clone()),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn contract_paused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "contract_paused"), admin.clone()), ());
}

/// topics: (event_name, admin)  data: ()
pub fn contract_unpaused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "contract_unpaused"), admin.clone()), ());
}

/// topics: (event_name, admin)  data: ()
pub fn approve_milestone_paused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "approve_milestone_paused"), admin.clone()),
        (),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn approve_milestone_unpaused(env: &Env, admin: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "approve_milestone_unpaused"),
            admin.clone(),
        ),
        (),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn contract_initialized(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "contract_initialized"), admin.clone()),
        (),
    );
}

/// topics: (event_name, admin)  data: progress_contract
pub fn progress_contract_updated(env: &Env, admin: &Address, progress_contract: &Address) {
    env.events().publish(
        (Symbol::new(env, "progress_contract_updated"), admin.clone()),
        progress_contract.clone(),
    );
}

/// topics: (event_name, admin, link)  data: (new_address, new_epoch)
///
/// Emitted by every `set_progress_contract` / `update_progress_contract` /
/// `set_registration_contract` / `update_registration_contract` call, in
/// addition to (not replacing) `progress_contract_updated`. `link`
/// identifies which peer pointer changed (`"progress_contract"` or
/// `"registration_contract"`). See `docs/WIRING_REGISTRY_DESIGN.md`.
pub fn wiring_updated(
    env: &Env,
    admin: &Address,
    link: &str,
    new_address: &Address,
    new_epoch: u32,
) {
    env.events().publish(
        (
            Symbol::new(env, WIRING_UPDATED),
            admin.clone(),
            Symbol::new(env, link),
        ),
        (new_address.clone(), new_epoch),
    );
}

/// Emitted when a player disputes a milestone (issue #471)
/// topics: (event_name, player_wallet)  data: (player_id, milestone_index, reason)
pub fn milestone_disputed(
    env: &Env,
    player_wallet: &Address,
    player_id: u64,
    milestone_index: u32,
    reason: &String,
) {
    env.events().publish(
        (
            Symbol::new(env, MILESTONE_DISPUTED),
            player_wallet.clone(),
        ),
        (player_id, milestone_index, reason.clone()),
    );
}

/// Emitted when an admin resolves a milestone dispute.
/// topics: (event_name, admin)  data: (player_id, milestone_index, upheld)
pub fn dispute_resolved(
    env: &Env,
    admin: &Address,
    player_id: u64,
    milestone_index: u32,
    upheld: bool,
) {
    env.events().publish(
        (Symbol::new(env, "dispute_resolved"), admin.clone()),
        (player_id, milestone_index, upheld),
    );
}

/// Emitted when a milestone is recorded but level advancement is skipped.
/// The milestone itself is still persisted; only the cross-contract
/// advance_level call is omitted. `reason` is either "AlreadyAtMaxLevel"
/// (player already at EliteTier) or "DiversityGateNotMet" (the attesting set
/// failed the affiliation-diversity or region-quorum requirement).
pub fn level_advancement_skipped(env: &Env, player_id: u64, reason: &String) {
    env.events().publish(
        (Symbol::new(env, LEVEL_ADVANCEMENT_SKIPPED), player_id),
        reason.clone(),
    );
}

/// Emitted when level advancement is skipped because the progress contract
/// address has not been configured.  Common during testing without a full
/// deployment.  In production this indicates a missing wiring step and the
/// indexer should alert on it.  The milestone is still persisted.
pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    env.events().publish(
        (Symbol::new(env, PROGRESS_CONTRACT_NOT_SET), player_id),
        (),
    );
}

/// Emitted on every accepted `attest_milestone` vote (including the
/// threshold-crossing one).
/// topics: (event_name, validator)  data: (player_id, evidence_hash, vote_count, threshold)
pub fn attestation_recorded(
    env: &Env,
    validator: &Address,
    player_id: u64,
    evidence_hash: &String,
    vote_count: u32,
    threshold: u32,
) {
    env.events().publish(
        (Symbol::new(env, ATTESTATION_RECORDED), validator.clone()),
        (player_id, evidence_hash.clone(), vote_count, threshold),
    );
}

/// Emitted when a sub-threshold claim's voting window has elapsed and a new
/// vote resets it to a fresh round, discarding all prior votes.
/// topics: (event_name, player_id)  data: (evidence_hash, new_round)
pub fn attestation_window_expired(
    env: &Env,
    player_id: u64,
    evidence_hash: &String,
    new_round: u32,
) {
    env.events().publish(
        (Symbol::new(env, ATTESTATION_WINDOW_EXPIRED), player_id),
        (evidence_hash.clone(), new_round),
    );
}

/// Emitted when `revoke_validator` retroactively strips a revoked
/// validator's contribution from still-pending (sub-threshold) claims.
/// topics: (event_name, admin)  data: (wallet, invalidated_count)
pub fn validator_votes_invalidated(
    env: &Env,
    admin: &Address,
    wallet: &Address,
    invalidated_count: u32,
) {
    env.events().publish(
        (
            Symbol::new(env, VALIDATOR_VOTES_INVALIDATED),
            admin.clone(),
        ),
        (wallet.clone(), invalidated_count),
    );
}

/// Emitted just before a ProgressCallFailed error is returned, so the
/// off-chain indexer can detect the failure by scanning transaction receipts.
/// Because ProgressCallFailed aborts the entire transaction, this event only
/// appears in the diagnostic stream — it is not committed to the ledger.
/// Payload is the raw error discriminant returned by try_advance_level.
pub fn progress_call_failed(env: &Env, player_id: u64, error_code: u32) {
    env.events().publish(
        (Symbol::new(env, PROGRESS_CALL_FAILED), player_id),
        error_code,
    );
}

/// Emitted by `restore_validator_record` when an admin re-extends an archived
/// or expired validator entry's TTL back to the core-identity policy value.
/// topics: (event_name, admin)  data: wallet
pub fn validator_record_restored(env: &Env, admin: &Address, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, VALIDATOR_RECORD_RESTORED), admin.clone()),
        wallet.clone(),
    );
}

/// Emitted by `restore_milestone_record` when an admin re-extends an archived
/// or expired milestone entry's TTL back to the core-identity policy value.
/// topics: (event_name, admin)  data: (player_id, index)
pub fn milestone_record_restored(env: &Env, admin: &Address, player_id: u64, index: u32) {
    env.events().publish(
        (Symbol::new(env, MILESTONE_RECORD_RESTORED), admin.clone()),
        (player_id, index),
    );
}

/// Emitted for each milestone flagged during a for-cause revocation cascade
/// (issue #1039).
///
/// topics: (event_name, validator)  data: (player_id, milestone_index)
pub fn milestone_flagged(
    env: &Env,
    validator: &Address,
    player_id: u64,
    milestone_index: u32,
) {
    env.events().publish(
        (Symbol::new(env, MILESTONE_FLAGGED), validator.clone()),
        (player_id, milestone_index),
    );
}

/// Emitted when an active validator clears a pending re-review flag via
/// `rereview_milestone` (issue #1039).
///
/// topics: (event_name, reviewer)  data: (player_id, milestone_index)
pub fn milestone_flag_cleared(env: &Env, reviewer: &Address, player_id: u64, milestone_index: u32) {
    env.events().publish(
        (Symbol::new(env, MILESTONE_FLAG_CLEARED), reviewer.clone()),
        (player_id, milestone_index),
    );
}

/// Emitted when a for-cause cascade sweep completes (all milestones flagged)
/// or when `continue_revocation_cascade` exhausts the remaining milestones
/// in a single call.
///
/// topics: (event_name, validator)  data: total_flagged_so_far
pub fn revocation_cascade_complete(env: &Env, validator: &Address, total_flagged: u32) {
    env.events().publish(
        (
            Symbol::new(env, REVOCATION_CASCADE_COMPLETE),
            validator.clone(),
        ),
        total_flagged,
    );
}

/// Emitted when a for-cause cascade sweep call reaches its per-call limit and
/// a continuation cursor is stored so the sweep can be resumed.
///
/// topics: (event_name, validator)  data: (next_cursor, flagged_this_call)
pub fn revocation_cascade_continued(
    env: &Env,
    validator: &Address,
    next_cursor: u32,
    flagged_this_call: u32,
) {
    env.events().publish(
        (
            Symbol::new(env, REVOCATION_CASCADE_CONTINUED),
            validator.clone(),
        ),
        (next_cursor, flagged_this_call),
    );
}

/// Emitted when a validator casts a vote on a jury-required milestone dispute.
///
/// topics: (event_name, validator)  data: (player_id, milestone_index, vote)
///
/// `vote` is `true` when the validator votes to uphold the original approval,
/// `false` when they vote to overturn it.  Matches the shape documented in
/// docs/DISPUTE_JURY.md and docs/EVENT_AUDIT.md.
pub fn dispute_vote_cast(
    env: &Env,
    player_id: u64,
    milestone_index: u32,
    validator: &Address,
    vote: bool,
) {
    env.events().publish(
        (Symbol::new(env, DISPUTE_VOTE_CAST), validator.clone()),
        (player_id, milestone_index, vote),
    );
}

/// Emitted when a jury dispute is tallied (closed with a final verdict).
///
/// topics: (event_name, player_id)  data: (milestone_index, upheld, votes_for, votes_against)
///
/// Matches the shape documented in docs/DISPUTE_JURY.md and docs/EVENT_AUDIT.md.
pub fn dispute_tallied(
    env: &Env,
    player_id: u64,
    milestone_index: u32,
    upheld: bool,
    votes_for: u32,
    votes_against: u32,
) {
    env.events().publish(
        (Symbol::new(env, DISPUTE_TALLIED), player_id),
        (milestone_index, upheld, votes_for, votes_against),
    );
}
