use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{contractevent, Address, Env};

// ── Typed contract events (issue #1370) ──────────────────────────────────────

/// Emitted by `advance_level` and `reset_player_level`.
#[contractevent]
pub struct ProgressUpdated {
    #[topic]
    pub updated_by: Address,
    pub player_id: u64,
    pub old_level: ProgressLevel,
    pub new_level: ProgressLevel,
}

/// Emitted by `reset_player_level` (admin dispute-resolution reset).
#[contractevent]
pub struct PlayerLevelReset {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
    pub old_level: ProgressLevel,
    pub target_level: ProgressLevel,
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

/// Emitted by `set_registration_contract` / `set_verification_contract` /
/// `set_scout_access_contract`.
#[contractevent]
pub struct WiringUpdated {
    #[topic]
    pub admin: Address,
    #[topic]
    pub link: soroban_sdk::Symbol,
    pub new_address: Address,
    pub new_epoch: u32,
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

/// Emitted by `restore_player_level_record`.
#[contractevent]
pub struct PlayerLevelRecordRestored {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
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

pub fn progress_updated(
    env: &Env,
    player_id: u64,
    old_level: &ProgressLevel,
    new_level: &ProgressLevel,
    updated_by: &Address,
    _milestone_ref: u32,
) {
    ProgressUpdated {
        updated_by: updated_by.clone(),
        player_id,
        old_level: old_level.clone(),
        new_level: new_level.clone(),
    }
    .emit(env);
}

pub fn player_level_reset(
    env: &Env,
    admin: &Address,
    player_id: u64,
    old_level: &ProgressLevel,
    target_level: &ProgressLevel,
) {
    PlayerLevelReset {
        admin: admin.clone(),
        player_id,
        old_level: old_level.clone(),
        target_level: target_level.clone(),
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

pub fn player_level_record_restored(env: &Env, admin: &Address, player_id: u64) {
    PlayerLevelRecordRestored {
        admin: admin.clone(),
        player_id,
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
