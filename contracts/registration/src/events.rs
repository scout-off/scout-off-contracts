use crate::types::MigrationRole;
use soroban_sdk::{contractevent, Address, Env};

// ── Typed contract events (issue #1370) ──────────────────────────────────────
//
// All events are defined as `#[contractevent]` structs so their schemas are
// included in the WASM contract spec. Generated TypeScript bindings can decode
// these events without ad-hoc topic-string parsing.
//
// Topic layout is preserved exactly for indexer backward compatibility:
//   • First topic field  → event name (Symbol, derived from struct name by SDK)
//   • Remaining #[topic] fields → additional indexed topics
//   • Non-#[topic] fields       → event data payload
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted by `register_player`.
#[contractevent]
pub struct PlayerRegistered {
    #[topic]
    pub wallet: Address,
    pub player_id: u64,
}

/// Emitted by `register_scout`.
#[contractevent]
pub struct ScoutRegistered {
    #[topic]
    pub wallet: Address,
    pub scout_id: u64,
}

/// Emitted by `update_profile`.
#[contractevent]
pub struct ProfileUpdated {
    #[topic]
    pub wallet: Address,
    pub player_id: u64,
}

/// Emitted by `deregister_player` (GDPR right-to-erasure).
#[contractevent]
pub struct PlayerDeregistered {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
}

/// Emitted by `deactivate_player`.
#[contractevent]
pub struct PlayerDeactivated {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
}

/// Emitted by `reactivate_player`.
#[contractevent]
pub struct PlayerReactivated {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
}

/// Emitted by `set_player_level` (called by the progress contract).
#[contractevent]
pub struct PlayerLevelSynced {
    #[topic]
    pub caller: Address,
    pub player_id: u64,
}

/// Emitted by `verify_scout`.
#[contractevent]
pub struct ScoutVerified {
    #[topic]
    pub wallet: Address,
    pub scout_id: u64,
}

/// Emitted by `deactivate_scout`.
#[contractevent]
pub struct ScoutDeactivated {
    #[topic]
    pub admin: Address,
    pub scout_id: u64,
}

/// Emitted by `reactivate_scout`.
#[contractevent]
pub struct ScoutReactivated {
    #[topic]
    pub admin: Address,
    pub scout_id: u64,
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

/// Emitted by `redeem_migration_ticket`.
#[contractevent]
pub struct MigrationRedeemed {
    #[topic]
    pub wallet: Address,
    pub role: MigrationRole,
    pub profile_id: u64,
    pub new_contract_hint: Address,
}

/// Emitted by `set_progress_contract`.
#[contractevent]
pub struct WiringUpdated {
    #[topic]
    pub admin: Address,
    #[topic]
    pub link: soroban_sdk::Symbol,
    pub new_address: Address,
    pub new_epoch: u32,
}

/// Emitted by `restore_player_record`.
#[contractevent]
pub struct PlayerRecordRestored {
    #[topic]
    pub admin: Address,
    pub player_id: u64,
}

/// Emitted by `restore_scout_record`.
#[contractevent]
pub struct ScoutRecordRestored {
    #[topic]
    pub admin: Address,
    pub scout_id: u64,
}

// ── Emit helpers ─────────────────────────────────────────────────────────────

pub fn player_registered(env: &Env, player_id: u64, wallet: &Address) {
    PlayerRegistered {
        wallet: wallet.clone(),
        player_id,
    }
    .emit(env);
}

pub fn scout_registered(env: &Env, scout_id: u64, wallet: &Address) {
    ScoutRegistered {
        wallet: wallet.clone(),
        scout_id,
    }
    .emit(env);
}

pub fn profile_updated(env: &Env, player_id: u64, wallet: &Address) {
    ProfileUpdated {
        wallet: wallet.clone(),
        player_id,
    }
    .emit(env);
}

pub fn player_deregistered(env: &Env, player_id: u64, admin: &Address) {
    PlayerDeregistered {
        admin: admin.clone(),
        player_id,
    }
    .emit(env);
}

pub fn player_deactivated(env: &Env, player_id: u64, admin: &Address) {
    PlayerDeactivated {
        admin: admin.clone(),
        player_id,
    }
    .emit(env);
}

pub fn player_reactivated(env: &Env, player_id: u64, admin: &Address) {
    PlayerReactivated {
        admin: admin.clone(),
        player_id,
    }
    .emit(env);
}

pub fn player_level_synced(env: &Env, player_id: u64, caller: &Address) {
    PlayerLevelSynced {
        caller: caller.clone(),
        player_id,
    }
    .emit(env);
}

pub fn scout_verified(env: &Env, scout_id: u64, wallet: &Address) {
    ScoutVerified {
        wallet: wallet.clone(),
        scout_id,
    }
    .emit(env);
}

pub fn scout_deactivated(env: &Env, scout_id: u64, admin: &Address) {
    ScoutDeactivated {
        admin: admin.clone(),
        scout_id,
    }
    .emit(env);
}

pub fn scout_reactivated(env: &Env, scout_id: u64, admin: &Address) {
    ScoutReactivated {
        admin: admin.clone(),
        scout_id,
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

pub fn migration_redeemed(
    env: &Env,
    wallet: &Address,
    role: &MigrationRole,
    profile_id: u64,
    new_contract_hint: &Address,
) {
    MigrationRedeemed {
        wallet: wallet.clone(),
        role: *role,
        profile_id,
        new_contract_hint: new_contract_hint.clone(),
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

pub fn player_record_restored(env: &Env, admin: &Address, player_id: u64) {
    PlayerRecordRestored {
        admin: admin.clone(),
        player_id,
    }
    .emit(env);
}

pub fn scout_record_restored(env: &Env, admin: &Address, scout_id: u64) {
    ScoutRecordRestored {
        admin: admin.clone(),
        scout_id,
    }
    .emit(env);
}
