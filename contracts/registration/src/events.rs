use crate::types::MigrationRole;
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{contractevent, Address, BytesN, Env, String, Symbol};

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
    pub level: scoutchain_shared_types::ProgressLevel,
    pub region: soroban_sdk::String,
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
    pub level: scoutchain_shared_types::ProgressLevel,
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
    .publish(env);
}

pub fn scout_registered(env: &Env, scout_id: u64, wallet: &Address) {
    ScoutRegistered {
        wallet: wallet.clone(),
        scout_id,
    }
    .publish(env);
}

pub fn profile_updated(env: &Env, player_id: u64, wallet: &Address) {
    ProfileUpdated {
        wallet: wallet.clone(),
        player_id,
    }
    .publish(env);
}

pub fn player_deregistered(
    env: &Env,
    player_id: u64,
    level: &ProgressLevel,
    region: &String,
    admin: &Address,
) {
    PlayerDeregistered {
        admin: admin.clone(),
        player_id,
        level: level.clone(),
        region: region.clone(),
    }
    .publish(env);
}

pub fn player_deactivated(env: &Env, player_id: u64, admin: &Address) {
    PlayerDeactivated {
        admin: admin.clone(),
        player_id,
    }
    .publish(env);
}

pub fn player_reactivated(env: &Env, player_id: u64, admin: &Address) {
    PlayerReactivated {
        admin: admin.clone(),
        player_id,
    }
    .publish(env);
}

/// `caller` is the progress contract address performing the level sync.
pub fn player_level_synced(env: &Env, player_id: u64, caller: &Address, level: &ProgressLevel) {
    PlayerLevelSynced {
        caller: caller.clone(),
        player_id,
        level: level.clone(),
    }
    .publish(env);
}

pub fn scout_verified(env: &Env, scout_id: u64, wallet: &Address) {
    ScoutVerified {
        wallet: wallet.clone(),
        scout_id,
    }
    .publish(env);
}

/// Emitted before `update_current_contract_wasm` — attributed to the old code version.
/// topics: (event_name, admin)  data: new_wasm_hash
pub fn contract_upgraded(env: &Env, admin: &Address, new_wasm_hash: &BytesN<32>) {
    env.events().publish(
        (Symbol::new(env, "contract_upgraded"), admin.clone()),
        new_wasm_hash.clone(),
    );
}
/// topics: (event_name, admin)  data: scout_id
pub fn scout_deactivated(env: &Env, scout_id: u64, admin: &Address) {
    ScoutDeactivated {
        admin: admin.clone(),
        scout_id,
    }
    .publish(env);
}

pub fn scout_reactivated(env: &Env, scout_id: u64, admin: &Address) {
    ScoutReactivated {
        admin: admin.clone(),
        scout_id,
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

/// topics: (event_name, admin)  data: (old_cooldown_secs, new_cooldown_secs)
pub fn reg_cooldown_updated(env: &Env, admin: &Address, old_cooldown: u64, new_cooldown: u64) {
    env.events().publish(
        (Symbol::new(env, "reg_cooldown_updated"), admin.clone()),
        (old_cooldown, new_cooldown),
    );
}

/// topics: (event_name, wallet)  data: (role, profile_id, new_contract_hint)
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

pub fn player_record_restored(env: &Env, admin: &Address, player_id: u64) {
    PlayerRecordRestored {
        admin: admin.clone(),
        player_id,
    }
    .publish(env);
}

pub fn scout_record_restored(env: &Env, admin: &Address, scout_id: u64) {
    ScoutRecordRestored {
        admin: admin.clone(),
        scout_id,
    }
    .publish(env);
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
