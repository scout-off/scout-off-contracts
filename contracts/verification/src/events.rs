#![allow(deprecated)]
use soroban_sdk::{Address, BytesN, Env, String, Symbol};

pub const MILESTONE_APPROVED: &str = "milestone_approved";
pub const VALIDATOR_REGISTERED: &str = "validator_registered";
pub const VALIDATOR_REVOKED: &str = "validator_revoked";
pub const VALIDATOR_REVOKED_FOR_CAUSE: &str = "validator_revoked_for_cause";
pub const CONTRACT_PAUSED: &str = "contract_paused";
pub const CONTRACT_UNPAUSED: &str = "contract_unpaused";
pub const CONTRACT_INITIALIZED: &str = "contract_initialized";
pub const PROGRESS_CONTRACT_UPDATED: &str = "progress_contract_updated";
pub const DISPUTE_RESOLVED: &str = "dispute_resolved";
pub const ADMIN_TRANSFER_PROPOSED: &str = "admin_transfer_proposed";
pub const ADMIN_TRANSFERRED: &str = "admin_transferred";
pub const CONTRACT_UPGRADED: &str = "contract_upgraded";

/// topics: (event_name, old_admin)  data: new_admin
pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (
            Symbol::new(env, ADMIN_TRANSFER_PROPOSED),
            old_admin.clone(),
        ),
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
        (
            Symbol::new(env, "milestone_approved"),
            validator.clone(),
        ),
        (player_id, milestone_index, description.clone(), evidence_hash.clone()),
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
        (Symbol::new(env, "validator_revoked_for_cause"), admin.clone()),
        (wallet.clone(), reason.clone()),
    );
}

/// topics: (event_name, admin)  data: wallet
pub fn validator_restored(env: &Env, admin: &Address, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "validator_restored"), admin.clone()),
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
        (Symbol::new(env, "validator_transferred"), admin.clone()),
        (old_wallet.clone(), new_wallet.clone()),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn contract_paused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "contract_paused"), admin.clone()),
        (),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn contract_unpaused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "contract_unpaused"), admin.clone()),
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

/// Emitted when a player disputes a milestone (issue #471)
/// topics: (event_name, player_wallet)  data: (player_id, milestone_index, reason)
pub fn milestone_disputed(env: &Env, player_wallet: &Address, player_id: u64, milestone_index: u32, reason: &String) {
    env.events().publish(
        (Symbol::new(env, "milestone_disputed"), player_wallet.clone()),
        (player_id, milestone_index, reason.clone()),
    );
}

/// Emitted when an admin resolves a milestone dispute.
/// topics: (event_name, admin)  data: (player_id, milestone_index, upheld)
pub fn dispute_resolved(env: &Env, admin: &Address, player_id: u64, milestone_index: u32, upheld: bool) {
    env.events().publish(
        (Symbol::new(env, "dispute_resolved"), admin.clone()),
        (player_id, milestone_index, upheld),
    );
}

/// Emitted when a milestone is recorded but level advancement is skipped because
/// the player is already at the maximum level (EliteTier).  The milestone itself
/// is still persisted; only the cross-contract advance_level call is omitted.
/// `reason` is always "AlreadyAtMaxLevel".
pub fn level_advancement_skipped(env: &Env, player_id: u64, reason: &String) {
    env.events().publish(
        (Symbol::new(env, "level_advancement_skipped"), player_id),
        reason.clone(),
    );
}

/// Emitted when level advancement is skipped because the progress contract
/// address has not been configured.  Common during testing without a full
/// deployment.  In production this indicates a missing wiring step and the
/// indexer should alert on it.  The milestone is still persisted.
pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    env.events().publish(
        (Symbol::new(env, "progress_contract_not_set"), player_id),
        (),
    );
}

/// Emitted just before a ProgressCallFailed error is returned, so the
/// off-chain indexer can detect the failure by scanning transaction receipts.
/// Because ProgressCallFailed aborts the entire transaction, this event only
/// appears in the diagnostic stream — it is not committed to the ledger.
/// Payload is the raw error discriminant returned by try_advance_level.
pub fn progress_call_failed(env: &Env, player_id: u64, error_code: u32) {
    env.events().publish(
        (Symbol::new(env, "progress_call_failed"), player_id),
        error_code,
    );
}

/// Emitted before `update_current_contract_wasm` — attributed to the old code version.
/// topics: (event_name, admin)  data: new_wasm_hash
pub fn contract_upgraded(env: &Env, admin: &Address, new_wasm_hash: &BytesN<32>) {
    env.events().publish(
        (Symbol::new(env, CONTRACT_UPGRADED), admin.clone()),
        new_wasm_hash.clone(),
    );
}
