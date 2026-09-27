#![allow(deprecated, dead_code)]
use crate::types::SubscriptionTier;
use soroban_sdk::{Address, BytesN, Env, Symbol};

pub const CONTRACT_INITIALIZED: &str = "contract_initialized";
pub const SCOUT_SUBSCRIBED: &str = "scout_subscribed";
pub const PLAYER_CONTACTED: &str = "player_contacted";
pub const TRIAL_OFFER_LOGGED: &str = "trial_offer_logged";
pub const TRIAL_OFFER_CONFIRMED: &str = "trial_offer_confirmed";
pub const TRIAL_OFFER_EXPIRED: &str = "trial_offer_expired";
pub const FEES_WITHDRAWN: &str = "fees_withdrawn";
pub const ADMIN_TRANSFERRED: &str = "admin_transferred";
pub const ADMIN_TRANSFER_PROPOSED: &str = "admin_transfer_proposed";
pub const CONTRACT_PAUSED: &str = "contract_paused";
pub const CONTRACT_UNPAUSED: &str = "contract_unpaused";
pub const SUBSCRIPTION_REFUNDED: &str = "subscription_refunded";
pub const PROGRESS_CONTRACT_UPDATED: &str = "progress_contract_updated";
pub const CONTRACT_UPGRADED: &str = "contract_upgraded";

/// topics: (event_name, admin)  data: admin
pub fn contract_initialized(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "contract_initialized"), admin.clone()),
        admin.clone(),
    );
}

/// topics: (event_name, scout)  data: (tier, fee_paid)
pub fn scout_subscribed(env: &Env, scout: &Address, tier: &SubscriptionTier, fee_paid: i128) {
    env.events().publish(
        (Symbol::new(env, "scout_subscribed"), scout.clone()),
        (tier.clone(), fee_paid),
    );
}

/// topics: (event_name, scout)  data: (player_id, fee_paid)
pub fn player_contacted(env: &Env, player_id: u64, scout: &Address, fee_paid: i128) {
    env.events().publish(
        (Symbol::new(env, "player_contacted"), scout.clone()),
        (player_id, fee_paid),
    );
}

/// topics: (event_name, scout)  data: player_id
pub fn trial_offer_logged(env: &Env, player_id: u64, scout: &Address) {
    env.events().publish(
        (Symbol::new(env, TRIAL_OFFER_LOGGED), scout.clone()),
        player_id,
    );
}

/// topics: (event_name, scout)  data: (player_id, index)
pub fn trial_offer_confirmed(env: &Env, player_id: u64, scout: &Address, index: u32) {
    env.events().publish(
        (Symbol::new(env, TRIAL_OFFER_CONFIRMED), scout.clone()),
        (player_id, index),
    );
}

/// topics: (event_name, scout)  data: (player_id, index)
pub fn trial_offer_expired(env: &Env, player_id: u64, scout: &Address, index: u32) {
    env.events().publish(
        (Symbol::new(env, TRIAL_OFFER_EXPIRED), scout.clone()),
        (player_id, index),
    );
}

/// topics: (event_name, admin)  data: (to, amount, timestamp)
pub fn fees_withdrawn(env: &Env, admin: &Address, to: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "fees_withdrawn"), admin.clone()),
        (to.clone(), amount, env.ledger().timestamp()),
    );
}

/// topics: (event_name, old_admin)  data: new_admin
pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "admin_transferred"), old_admin.clone()),
        new_admin.clone(),
    );
}

/// topics: (event_name, old_admin)  data: new_admin
pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, ADMIN_TRANSFER_PROPOSED), old_admin.clone()),
        new_admin.clone(),
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

/// topics: (event_name, scout)  data: (tier, subscribed_at, expires_at)
pub fn subscription_created(
    env: &Env,
    scout: &Address,
    tier: &SubscriptionTier,
    subscribed_at: u64,
    expires_at: u64,
) {
    env.events().publish(
        (Symbol::new(env, "subscription_created"), scout.clone()),
        (tier.clone(), subscribed_at, expires_at),
    );
}

/// topics: (event_name, scout)  data: (tier, subscribed_at, expires_at)
pub fn subscription_renewed(
    env: &Env,
    scout: &Address,
    tier: &SubscriptionTier,
    subscribed_at: u64,
    expires_at: u64,
) {
    env.events().publish(
        (Symbol::new(env, "subscription_renewed"), scout.clone()),
        (tier.clone(), subscribed_at, expires_at),
    );
}

/// topics: (event_name, scout)  data: amount
pub fn subscription_refunded(env: &Env, scout: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "subscription_refunded"), scout.clone()),
        amount,
    );
}

/// topics: (event_name, admin)  data: progress_contract
pub fn progress_contract_updated(env: &Env, admin: &Address, progress_contract: &Address) {
    env.events().publish(
        (Symbol::new(env, "progress_contract_updated"), admin.clone()),
        progress_contract.clone(),
    );
}

/// topics: (event_name, admin)  data: (old_config, new_config)
pub fn fee_config_updated(
    env: &Env,
    admin: &Address,
    old_config: &crate::types::FeeConfig,
    new_config: &crate::types::FeeConfig,
) {
    env.events().publish(
        (Symbol::new(env, "fee_config_updated"), admin.clone()),
        (old_config.clone(), new_config.clone()),
    );
}

/// Emitted when confirm_trial_offer is skipped because the progress contract
/// address has not been configured.  Indicates missing wiring; the indexer
/// should alert on this event in production.
pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    env.events().publish(
        (Symbol::new(env, "progress_contract_not_set"), player_id),
        (),
    );
}

/// Emitted just before a ProgressCallFailed error is returned from
/// confirm_trial_offer, so indexers scanning transaction receipts can detect
/// the failure without parsing raw error codes.  Because ProgressCallFailed
/// aborts the whole transaction, this event only appears in the diagnostic
/// stream — not in committed ledger events.
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
