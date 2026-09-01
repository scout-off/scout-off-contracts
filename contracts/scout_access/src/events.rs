#![allow(deprecated, dead_code)]
use crate::types::SubscriptionTier;
use soroban_sdk::{Address, Env, Symbol};

pub const CONTRACT_INITIALIZED: &str = "contract_initialized";
pub const SCOUT_SUBSCRIBED: &str = "scout_subscribed";
pub const PLAYER_CONTACTED: &str = "player_contacted";
pub const TRIAL_OFFER_LOGGED: &str = "trial_offer_logged";
pub const TRIAL_OFFER_CONFIRMED: &str = "trial_offer_confirmed";
pub const TRIAL_OFFER_EXPIRED: &str = "trial_offer_expired";
pub const TRIAL_ESCROW_ADMIN_REFUNDED: &str = "trial_escrow_admin_refunded";
pub const FEES_WITHDRAWN: &str = "fees_withdrawn";
pub const ADMIN_TRANSFERRED: &str = "admin_transferred";
pub const ADMIN_TRANSFER_PROPOSED: &str = "admin_transfer_proposed";
pub const CONTRACT_PAUSED: &str = "contract_paused";
pub const CONTRACT_UNPAUSED: &str = "contract_unpaused";
pub const SUBSCRIPTION_REFUNDED: &str = "subscription_refunded";
pub const PROGRESS_CONTRACT_UPDATED: &str = "progress_contract_updated";
pub const REGISTRATION_CONTRACT_UPDATED: &str = "registration_contract_updated";
pub const FEE_CONFIG_PROPOSED: &str = "fee_config_proposed";
pub const FEE_CONFIG_UPDATED: &str = "fee_config_updated";
pub const FEE_CONFIG_DELAY_BYPASSED: &str = "fee_config_delay_bypassed";
pub const WIRING_UPDATED: &str = "wiring_updated";
pub const EVIDENCE_ACCESS_GRANTED: &str = "evidence_access_granted";
pub const EVIDENCE_ACCESS_REVOKED: &str = "evidence_access_revoked";

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

/// topics: (event_name, to)  data: (player_id, index, amount)
pub fn trial_escrow_admin_refunded(env: &Env, player_id: u64, index: u32, to: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, TRIAL_ESCROW_ADMIN_REFUNDED), to.clone()),
        (player_id, index, amount),
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
    env.events()
        .publish((Symbol::new(env, "contract_paused"), admin.clone()), ());
}

/// topics: (event_name, admin)  data: ()
pub fn contract_unpaused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "contract_unpaused"), admin.clone()), ());
}

pub const PAY_TO_CONTACT_PAUSED: &str = "pay_to_contact_paused";
pub const PAY_TO_CONTACT_UNPAUSED: &str = "pay_to_contact_unpaused";

/// topics: (event_name, admin)  data: ()
pub fn pay_to_contact_paused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "pay_to_contact_paused"), admin.clone()),
        (),
    );
}

/// topics: (event_name, admin)  data: ()
pub fn pay_to_contact_unpaused(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "pay_to_contact_unpaused"), admin.clone()),
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

/// topics: (event_name, admin)  data: registration_contract
pub fn registration_contract_updated(env: &Env, admin: &Address, registration_contract: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "registration_contract_updated"),
            admin.clone(),
        ),
        registration_contract.clone(),
    );
}

/// topics: (event_name, admin, link)  data: (new_address, new_epoch)
///
/// Emitted by every `set_progress_contract` / `update_progress_contract` /
/// `set_registration_contract` call, in addition to (not replacing)
/// `progress_contract_updated` / `registration_contract_updated`. `link`
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

/// topics: (event_name, admin)  data: (proposed_config, proposed_at)
pub fn fee_config_proposed(
    env: &Env,
    admin: &Address,
    proposed_config: &crate::types::FeeConfig,
    proposed_at: u64,
) {
    env.events().publish(
        (Symbol::new(env, "fee_config_proposed"), admin.clone()),
        (proposed_config.clone(), proposed_at),
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

/// topics: (event_name, admin)  data: (old_config, new_config)
///
/// Emitted only by `update_fee_config`, alongside (never instead of)
/// `fee_config_updated`, so indexers/auditors can tell — purely from the
/// event stream — that this particular fee change bypassed the 7-day
/// `propose_fee_config` / `activate_fee_config` delay (see
/// docs/FEE_CONFIG_PROPOSAL_DESIGN.md). A `fee_config_updated` event that is
/// *not* accompanied by this event in the same transaction, and is also not
/// accompanied by a same-transaction `fee_config_proposed`, was activated via
/// `activate_fee_config` after the full delay elapsed.
pub fn fee_config_delay_bypassed(
    env: &Env,
    admin: &Address,
    old_config: &crate::types::FeeConfig,
    new_config: &crate::types::FeeConfig,
) {
    env.events().publish(
        (Symbol::new(env, "fee_config_delay_bypassed"), admin.clone()),
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

pub const AUTO_RENEW_SET: &str = "auto_renew_set";
pub const SUBSCRIPTION_AUTO_RENEWED: &str = "subscription_auto_renewed";
pub const EVIDENCE_ACCESS_GRANTED: &str = "evidence_access_granted";
pub const EVIDENCE_ACCESS_REVOKED: &str = "evidence_access_revoked";

/// topics: (event_name, scout)  data: enabled
pub fn auto_renew_set(env: &Env, scout: &Address, enabled: bool) {
    env.events()
        .publish((Symbol::new(env, AUTO_RENEW_SET), scout.clone()), enabled);
}

/// topics: (event_name, scout)  data: (tier, subscribed_at, expires_at)
///
/// Emitted when `renew_if_due` successfully renews a scout's subscription.
pub fn subscription_auto_renewed(
    env: &Env,
    scout: &Address,
    tier: &SubscriptionTier,
    subscribed_at: u64,
    expires_at: u64,
) {
    env.events().publish(
        (Symbol::new(env, SUBSCRIPTION_AUTO_RENEWED), scout.clone()),
        (tier.clone(), subscribed_at, expires_at),
    );
}

/// Emitted by `restore_subscription_record` when an admin re-extends an
/// archived or expired subscription entry's TTL back to the policy value.
/// topics: (event_name, admin)  data: scout
pub fn subscription_record_restored(env: &Env, admin: &Address, scout: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "subscription_record_restored"),
            admin.clone(),
        ),
        scout.clone(),
    );
}

/// topics: (event_name, scout)  data: (player_id, tier_at_grant)
///
/// Emitted exactly once, atomically with a successful `pay_to_contact` /
/// `batch_contact_players` call, when an `EvidenceAccessGrant` is written.
/// The frontend/backend key-wrapping service watches this event to deliver
/// a viewer-specific wrapped decryption key — see `docs/EVIDENCE_PRIVACY.md`.
pub fn evidence_access_granted(env: &Env, player_id: u64, scout: &Address, tier: &SubscriptionTier) {
    env.events().publish(
        (Symbol::new(env, EVIDENCE_ACCESS_GRANTED), scout.clone()),
        (player_id, tier.clone()),
    );
}

/// topics: (event_name, scout)  data: (player_id, admin)
///
/// Emitted by `admin_revoke_evidence_access`. This only signals that the
/// off-chain key-wrapping service should stop honoring future key-wrap
/// requests for this (player_id, scout) pair — it cannot revoke a wrapped
/// key that was already delivered before this event.
pub fn evidence_access_revoked(env: &Env, player_id: u64, scout: &Address, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, EVIDENCE_ACCESS_REVOKED), scout.clone()),
        (player_id, admin.clone()),
    );
}

pub const REGIONAL_CONTACT_LIMIT_SET: &str = "regional_contact_limit_set";

/// topics: (event_name, admin)  data: (region, limit)
///
/// Emitted when an admin sets or updates a per-region Pro-tier contact limit override.
pub fn regional_contact_limit_set(env: &Env, admin: &Address, region: &soroban_sdk::String, limit: u32) {
    env.events().publish(
        (Symbol::new(env, REGIONAL_CONTACT_LIMIT_SET), admin.clone()),
        (region.clone(), limit),
    );
}
