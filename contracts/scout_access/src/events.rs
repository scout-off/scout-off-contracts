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
pub const PLAYER_PURGE_STARTED: &str = "player_purge_started";
pub const PLAYER_DATA_PURGED: &str = "player_data_purged";
pub const PLAYER_PURGE_COMPLETED: &str = "player_purge_completed";

/// topics: (event_name, admin)  data: admin
pub fn contract_initialized(env: &Env, admin: &Address) {
    ContractInitialized {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn scout_subscribed(env: &Env, scout: &Address, tier: &SubscriptionTier, fee_paid: i128) {
    ScoutSubscribed {
        scout: scout.clone(),
        tier: tier.clone(),
        fee_paid,
    }
    .publish(env);
}

pub fn player_contacted(env: &Env, player_id: u64, scout: &Address, fee_paid: i128) {
    PlayerContacted {
        scout: scout.clone(),
        player_id,
        fee_paid,
    }
    .publish(env);
}

pub fn trial_offer_logged(env: &Env, player_id: u64, scout: &Address) {
    TrialOfferLogged {
        scout: scout.clone(),
        player_id,
    }
    .publish(env);
}

pub fn trial_offer_confirmed(env: &Env, player_id: u64, scout: &Address, index: u32) {
    TrialOfferConfirmed {
        scout: scout.clone(),
        player_id,
        index,
    }
    .publish(env);
}

pub fn trial_offer_expired(env: &Env, player_id: u64, scout: &Address, index: u32) {
    TrialOfferExpired {
        scout: scout.clone(),
        player_id,
        index,
    }
    .publish(env);
}

pub fn trial_escrow_orphan_rescued(
    env: &Env,
    player_id: u64,
    index: u32,
    to: &Address,
    amount: i128,
) {
    TrialEscrowOrphanRescued {
        to: to.clone(),
        player_id,
        index,
        amount,
    }
    .publish(env);
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
    .publish(env);
}

pub fn fees_withdrawn(env: &Env, admin: &Address, to: &Address, amount: i128) {
    FeesWithdrawn {
        admin: admin.clone(),
        to: to.clone(),
        amount,
        timestamp: env.ledger().timestamp(),
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

pub fn admin_transfer_proposed(env: &Env, old_admin: &Address, new_admin: &Address) {
    AdminTransferProposed {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
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

pub fn pay_to_contact_paused(env: &Env, admin: &Address) {
    PayToContactPaused {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn pay_to_contact_unpaused(env: &Env, admin: &Address) {
    PayToContactUnpaused {
        admin: admin.clone(),
    }
    .publish(env);
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
    .publish(env);
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
    .publish(env);
}

/// topics: (event_name, scout)  data: (admin, amount)
pub fn subscription_refunded(env: &Env, scout: &Address, admin: &Address, amount: i128) {
    SubscriptionRefunded {
        scout: scout.clone(),
        admin: admin.clone(),
        amount,
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

pub fn registration_contract_updated(env: &Env, admin: &Address, registration_contract: &Address) {
    RegistrationContractUpdated {
        admin: admin.clone(),
        registration_contract: registration_contract.clone(),
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
    .publish(env);
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
    .publish(env);
}

/// topics: (event_name, admin)  data: ()
///
/// Emitted when a pending fee config proposal is cancelled via
/// `cancel_fee_config_proposal`, or cleared implicitly by `update_fee_config`.
pub fn fee_config_proposal_cancelled(env: &Env, admin: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "fee_config_proposal_cancelled"),
            admin.clone(),
        ),
        (),
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
    old_config: &FeeConfig,
    new_config: &FeeConfig,
) {
    FeeConfigDelayBypassed {
        admin: admin.clone(),
        old_config: old_config.clone(),
        new_config: new_config.clone(),
    }
    .publish(env);
}

pub fn progress_contract_not_set(env: &Env, player_id: u64) {
    ProgressContractNotSet { player_id }.publish(env);
}

pub fn progress_call_failed(env: &Env, player_id: u64, error_code: u32) {
    ProgressCallFailed {
        player_id,
        error_code,
    }
    .publish(env);
}

pub fn auto_renew_set(env: &Env, scout: &Address, enabled: bool) {
    AutoRenewSet {
        scout: scout.clone(),
        enabled,
    }
    .publish(env);
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
    .publish(env);
}

pub fn subscription_record_restored(env: &Env, admin: &Address, scout: &Address) {
    SubscriptionRecordRestored {
        admin: admin.clone(),
        scout: scout.clone(),
    }
    .publish(env);
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
    .publish(env);
}

pub fn evidence_access_revoked(env: &Env, player_id: u64, scout: &Address, admin: &Address) {
    EvidenceAccessRevoked {
        scout: scout.clone(),
        player_id,
        admin: admin.clone(),
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

pub const EVIDENCE_ACCESS_RENEWED: &str = "evidence_access_renewed";
pub const EVIDENCE_ACCESS_REINSTATED: &str = "evidence_access_reinstated";

/// topics: (event_name, scout)  data: (player_id, expires_at, fee_stroops)
///
/// Emitted by `renew_evidence_access` after the scout paid to extend an
/// expired grant.
pub fn evidence_access_renewed(
    env: &Env,
    player_id: u64,
    scout: &Address,
    expires_at: u64,
    fee_stroops: i128,
) {
    env.events().publish(
        (Symbol::new(env, EVIDENCE_ACCESS_RENEWED), scout.clone()),
        (player_id, expires_at, fee_stroops),
    );
}

/// topics: (event_name, scout)  data: (player_id, player, expires_at)
///
/// Emitted by `reinstate_evidence_access` when a player re-approves a scout
/// whose grant they had previously revoked.
pub fn evidence_access_reinstated(
    env: &Env,
    player_id: u64,
    scout: &Address,
    player: &Address,
    expires_at: u64,
) {
    env.events().publish(
        (Symbol::new(env, EVIDENCE_ACCESS_REINSTATED), scout.clone()),
        (player_id, player.clone(), expires_at),
    );
}

pub const EVIDENCE_ACCESS_REVOKED_BY_PLAYER: &str = "evidence_revoked_by_player";

/// topics: (event_name, scout)  data: (player_id, player)
///
/// Emitted by `revoke_evidence_access` (player-initiated). Like
/// `evidence_access_revoked` (admin path), this only instructs the off-chain
/// key-wrapping service to stop honoring *future* key-wrap requests for
/// this (player_id, scout) pair — it cannot claw back a wrapped key already
/// delivered before this event.
pub fn evidence_access_revoked_by_player(
    env: &Env,
    player_id: u64,
    scout: &Address,
    player: &Address,
) {
    env.events().publish(
        (
            Symbol::new(env, EVIDENCE_ACCESS_REVOKED_BY_PLAYER),
            scout.clone(),
        ),
        (player_id, player.clone()),
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

/// topics: (event_name, admin)  data: ()
///
/// Emitted by `open_migration_window`. While the window is open the
/// `admin_seed_*` entrypoints are enabled, so production monitoring should
/// alert on this event (see docs/EVENT_REFERENCE.md and docs/RUNBOOK.md).
pub fn migration_window_opened(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, MIGRATION_WINDOW_OPENED), admin.clone()),
        (),
    );
}

/// topics: (event_name, admin)  data: ()
///
/// Emitted by `close_migration_window`, which disables the `admin_seed_*`
/// entrypoints again.
pub fn migration_window_closed(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, MIGRATION_WINDOW_CLOSED), admin.clone()),
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
