use crate::types::{FeeConfig, SubscriptionTier};
use soroban_sdk::{contractevent, Address, Env};

// ── Typed contract events (issue #1370) ──────────────────────────────────────

/// Emitted by `initialize` / `__constructor`.
#[contractevent]
pub struct ContractInitialized {
    #[topic]
    pub admin: Address,
}

/// Emitted by `subscribe` (legacy, alongside `SubscriptionCreated` or `SubscriptionRenewed`).
#[contractevent]
pub struct ScoutSubscribed {
    #[topic]
    pub scout: Address,
    pub tier: SubscriptionTier,
    pub fee_paid: i128,
}

/// Emitted by `subscribe` when the scout purchases their first subscription.
#[contractevent]
pub struct SubscriptionCreated {
    #[topic]
    pub scout: Address,
    pub tier: SubscriptionTier,
    pub subscribed_at: u64,
    pub expires_at: u64,
}

/// Emitted by `subscribe` when the scout renews or upgrades an existing subscription.
#[contractevent]
pub struct SubscriptionRenewed {
    #[topic]
    pub scout: Address,
    pub tier: SubscriptionTier,
    pub subscribed_at: u64,
    pub expires_at: u64,
}

/// Emitted when a subscription is refunded.
#[contractevent]
pub struct SubscriptionRefunded {
    #[topic]
    pub scout: Address,
    pub amount: i128,
}

/// Emitted by `pay_to_contact`.
#[contractevent]
pub struct PlayerContacted {
    #[topic]
    pub scout: Address,
    pub player_id: u64,
    pub fee_paid: i128,
}

/// Emitted by `log_trial_offer` (escrows a fee; does not advance the level).
#[contractevent]
pub struct TrialOfferLogged {
    #[topic]
    pub scout: Address,
    pub player_id: u64,
}

/// Emitted by `confirm_trial_offer` (player confirms before expiry).
#[contractevent]
pub struct TrialOfferConfirmed {
    #[topic]
    pub scout: Address,
    pub player_id: u64,
    pub index: u32,
}

/// Emitted when a trial offer confirmation window elapses; escrow refunded.
#[contractevent]
pub struct TrialOfferExpired {
    #[topic]
    pub scout: Address,
    pub player_id: u64,
    pub index: u32,
}

/// Emitted when an admin refunds a trial escrow directly.
#[contractevent]
pub struct TrialEscrowAdminRefunded {
    #[topic]
    pub to: Address,
    pub player_id: u64,
    pub index: u32,
    pub amount: i128,
}

/// Emitted by `withdraw_fees`.
#[contractevent]
pub struct FeesWithdrawn {
    #[topic]
    pub admin: Address,
    pub to: Address,
    pub amount: i128,
    pub timestamp: u64,
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

/// Emitted by `pause_pay_to_contact`.
#[contractevent]
pub struct PayToContactPaused {
    #[topic]
    pub admin: Address,
}

/// Emitted by `unpause_pay_to_contact`.
#[contractevent]
pub struct PayToContactUnpaused {
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

/// Emitted by `set_registration_contract`.
#[contractevent]
pub struct RegistrationContractUpdated {
    #[topic]
    pub admin: Address,
    pub registration_contract: Address,
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

/// Emitted by `propose_fee_config`.
#[contractevent]
pub struct FeeConfigProposed {
    #[topic]
    pub admin: Address,
    pub proposed_config: FeeConfig,
    pub proposed_at: u64,
}

/// Emitted by `update_fee_config` and `activate_fee_config`.
#[contractevent]
pub struct FeeConfigUpdated {
    #[topic]
    pub admin: Address,
    pub old_config: FeeConfig,
    pub new_config: FeeConfig,
}

/// Emitted alongside `FeeConfigUpdated` when `update_fee_config` bypasses the delay.
#[contractevent]
pub struct FeeConfigDelayBypassed {
    #[topic]
    pub admin: Address,
    pub old_config: FeeConfig,
    pub new_config: FeeConfig,
}

/// Emitted when confirm_trial_offer is skipped because the progress contract
/// address has not been configured.
#[contractevent]
pub struct ProgressContractNotSet {
    #[topic]
    pub player_id: u64,
}

/// Emitted just before a `ProgressCallFailed` error is returned.
#[contractevent]
pub struct ProgressCallFailed {
    #[topic]
    pub player_id: u64,
    pub error_code: u32,
}

/// Emitted by `set_auto_renew`.
#[contractevent]
pub struct AutoRenewSet {
    #[topic]
    pub scout: Address,
    pub enabled: bool,
}

/// Emitted when `renew_if_due` successfully renews a scout's subscription.
#[contractevent]
pub struct SubscriptionAutoRenewed {
    #[topic]
    pub scout: Address,
    pub tier: SubscriptionTier,
    pub subscribed_at: u64,
    pub expires_at: u64,
}

/// Emitted by `restore_subscription_record`.
#[contractevent]
pub struct SubscriptionRecordRestored {
    #[topic]
    pub admin: Address,
    pub scout: Address,
}

/// Emitted by `pay_to_contact` / `batch_contact_players` when a grant is issued.
#[contractevent]
pub struct EvidenceAccessGranted {
    #[topic]
    pub scout: Address,
    pub player_id: u64,
    pub tier: SubscriptionTier,
}

/// Emitted by `admin_revoke_evidence_access`.
#[contractevent]
pub struct EvidenceAccessRevoked {
    #[topic]
    pub scout: Address,
    pub player_id: u64,
    pub admin: Address,
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

pub fn scout_subscribed(env: &Env, scout: &Address, tier: &SubscriptionTier, fee_paid: i128) {
    ScoutSubscribed {
        scout: scout.clone(),
        tier: tier.clone(),
        fee_paid,
    }
    .emit(env);
}

pub fn player_contacted(env: &Env, player_id: u64, scout: &Address, fee_paid: i128) {
    PlayerContacted {
        scout: scout.clone(),
        player_id,
        fee_paid,
    }
    .emit(env);
}

pub fn trial_offer_logged(env: &Env, player_id: u64, scout: &Address) {
    TrialOfferLogged {
        scout: scout.clone(),
        player_id,
    }
    .emit(env);
}

pub fn trial_offer_confirmed(env: &Env, player_id: u64, scout: &Address, index: u32) {
    TrialOfferConfirmed {
        scout: scout.clone(),
        player_id,
        index,
    }
    .emit(env);
}

pub fn trial_offer_expired(env: &Env, player_id: u64, scout: &Address, index: u32) {
    TrialOfferExpired {
        scout: scout.clone(),
        player_id,
        index,
    }
    .emit(env);
}

pub fn trial_escrow_admin_refunded(
    env: &Env,
    player_id: u64,
    index: u32,
    to: &Address,
    amount: i128,
) {
    TrialEscrowAdminRefunded {
        to: to.clone(),
        player_id,
        index,
        amount,
    }
    .emit(env);
}

pub fn fees_withdrawn(env: &Env, admin: &Address, to: &Address, amount: i128) {
    FeesWithdrawn {
        admin: admin.clone(),
        to: to.clone(),
        amount,
        timestamp: env.ledger().timestamp(),
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

pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    AdminTransferProposed {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
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

pub fn pay_to_contact_paused(env: &Env, admin: &Address) {
    PayToContactPaused {
        admin: admin.clone(),
    }
    .emit(env);
}

pub fn pay_to_contact_unpaused(env: &Env, admin: &Address) {
    PayToContactUnpaused {
        admin: admin.clone(),
    }
    .emit(env);
}

pub fn subscription_created(
    env: &Env,
    scout: &Address,
    tier: &SubscriptionTier,
    subscribed_at: u64,
    expires_at: u64,
) {
    SubscriptionCreated {
        scout: scout.clone(),
        tier: tier.clone(),
        subscribed_at,
        expires_at,
    }
    .emit(env);
}

pub fn subscription_renewed(
    env: &Env,
    scout: &Address,
    tier: &SubscriptionTier,
    subscribed_at: u64,
    expires_at: u64,
) {
    SubscriptionRenewed {
        scout: scout.clone(),
        tier: tier.clone(),
        subscribed_at,
        expires_at,
    }
    .emit(env);
}

pub fn subscription_refunded(env: &Env, scout: &Address, amount: i128) {
    SubscriptionRefunded {
        scout: scout.clone(),
        amount,
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

pub fn registration_contract_updated(env: &Env, admin: &Address, registration_contract: &Address) {
    RegistrationContractUpdated {
        admin: admin.clone(),
        registration_contract: registration_contract.clone(),
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

pub fn fee_config_proposed(
    env: &Env,
    admin: &Address,
    proposed_config: &FeeConfig,
    proposed_at: u64,
) {
    FeeConfigProposed {
        admin: admin.clone(),
        proposed_config: proposed_config.clone(),
        proposed_at,
    }
    .emit(env);
}

pub fn fee_config_updated(
    env: &Env,
    admin: &Address,
    old_config: &FeeConfig,
    new_config: &FeeConfig,
) {
    FeeConfigUpdated {
        admin: admin.clone(),
        old_config: old_config.clone(),
        new_config: new_config.clone(),
    }
    .emit(env);
}

pub fn fee_config_delay_bypassed(
    env: &Env,
    admin: &Address,
    old_config: &FeeConfig,
    new_config: &FeeConfig,
) {
    FeeConfigDelayBypassed {
        admin: admin.clone(),
        old_config: old_config.clone(),
        new_config: new_config.clone(),
    }
    .emit(env);
}

pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    ProgressContractNotSet { player_id }.emit(env);
}

pub fn progress_call_failed(env: &Env, player_id: u64, error_code: u32) {
    ProgressCallFailed {
        player_id,
        error_code,
    }
    .emit(env);
}

pub fn auto_renew_set(env: &Env, scout: &Address, enabled: bool) {
    AutoRenewSet {
        scout: scout.clone(),
        enabled,
    }
    .emit(env);
}

pub fn subscription_auto_renewed(
    env: &Env,
    scout: &Address,
    tier: &SubscriptionTier,
    subscribed_at: u64,
    expires_at: u64,
) {
    SubscriptionAutoRenewed {
        scout: scout.clone(),
        tier: tier.clone(),
        subscribed_at,
        expires_at,
    }
    .emit(env);
}

pub fn subscription_record_restored(env: &Env, admin: &Address, scout: &Address) {
    SubscriptionRecordRestored {
        admin: admin.clone(),
        scout: scout.clone(),
    }
    .emit(env);
}

pub fn evidence_access_granted(
    env: &Env,
    player_id: u64,
    scout: &Address,
    tier: &SubscriptionTier,
) {
    EvidenceAccessGranted {
        scout: scout.clone(),
        player_id,
        tier: tier.clone(),
    }
    .emit(env);
}

pub fn evidence_access_revoked(env: &Env, player_id: u64, scout: &Address, admin: &Address) {
    EvidenceAccessRevoked {
        scout: scout.clone(),
        player_id,
        admin: admin.clone(),
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
