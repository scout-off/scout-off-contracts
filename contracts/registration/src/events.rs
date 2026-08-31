#![allow(deprecated, dead_code)]
use soroban_sdk::{Address, Env, Symbol};

use crate::types::MigrationRole;

pub const PLAYER_REGISTERED: &str = "player_registered";
pub const SCOUT_REGISTERED: &str = "scout_registered";
pub const PROFILE_UPDATED: &str = "profile_updated";
pub const PLAYER_DEREGISTERED: &str = "player_deregistered";
pub const PLAYER_DEACTIVATED: &str = "player_deactivated";
pub const PLAYER_REACTIVATED: &str = "player_reactivated";
pub const PLAYER_LEVEL_SYNCED: &str = "player_level_synced";
pub const SCOUT_VERIFIED: &str = "scout_verified";
pub const SCOUT_DEACTIVATED: &str = "scout_deactivated";
pub const SCOUT_REACTIVATED: &str = "scout_reactivated";
pub const ADMIN_TRANSFER_PROPOSED: &str = "admin_transfer_proposed";
pub const ADMIN_TRANSFERRED: &str = "admin_transferred";
pub const MIGRATION_REDEEMED: &str = "migration_redeemed";
pub const WIRING_UPDATED: &str = "wiring_updated";

/// topics: (event_name, admin, link)  data: (new_address, new_epoch)
///
/// Emitted by `set_progress_contract`. `link` is always
/// `"progress_contract"` (the contract's only wiring pointer); the argument
/// exists so this event's shape matches the other three contracts'
/// `wiring_updated` events. See `docs/WIRING_REGISTRY_DESIGN.md`.
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

/// topics: (event_name, wallet)  data: player_id
pub fn player_registered(env: &Env, player_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_registered"), wallet.clone()),
        player_id,
    );
}

pub fn player_deregistered(env: &Env, player_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_deregistered"), wallet.clone()),
        player_id,
    );
}

/// topics: (event_name, wallet)  data: scout_id
pub fn scout_registered(env: &Env, scout_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "scout_registered"), wallet.clone()),
        scout_id,
    );
}

/// topics: (event_name, wallet)  data: player_id
pub fn profile_updated(env: &Env, player_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "profile_updated"), wallet.clone()),
        player_id,
    );
}

/// topics: (event_name, admin)  data: player_id
pub fn player_deregistered(env: &Env, player_id: u64, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_deregistered"), admin.clone()),
        player_id,
    );
}

/// topics: (event_name, admin)  data: player_id
pub fn player_deactivated(env: &Env, player_id: u64, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_deactivated"), admin.clone()),
        player_id,
    );
}

/// topics: (event_name, admin)  data: player_id
pub fn player_reactivated(env: &Env, player_id: u64, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_reactivated"), admin.clone()),
        player_id,
    );
}

/// topics: (event_name, caller)  data: player_id
/// `caller` is the progress contract address performing the level sync.
pub fn player_level_synced(env: &Env, player_id: u64, caller: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_level_synced"), caller.clone()),
        player_id,
    );
}

/// topics: (event_name, wallet)  data: scout_id
pub fn scout_verified(env: &Env, scout_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "scout_verified"), wallet.clone()),
        scout_id,
    );
}

/// topics: (event_name, admin)  data: scout_id
pub fn scout_deactivated(env: &Env, scout_id: u64, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, SCOUT_DEACTIVATED), admin.clone()),
        scout_id,
    );
}

/// topics: (event_name, admin)  data: scout_id
pub fn scout_reactivated(env: &Env, scout_id: u64, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, SCOUT_REACTIVATED), admin.clone()),
        scout_id,
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
    env.events().publish(
        (Symbol::new(env, MIGRATION_REDEEMED), wallet.clone()),
        (*role, profile_id, new_contract_hint.clone()),
    );
}

/// topics: (event_name, admin)  data: player_id
/// Emitted by `restore_player_record` when an admin re-extends an archived or
/// expired player profile's TTL back to the core-identity policy value.
pub fn player_record_restored(env: &Env, admin: &Address, player_id: u64) {
    env.events().publish(
        (Symbol::new(env, "player_record_restored"), admin.clone()),
        player_id,
    );
}

/// topics: (event_name, admin)  data: scout_id
/// Emitted by `restore_scout_record` when an admin re-extends an archived or
/// expired scout profile's TTL back to the core-identity policy value.
pub fn scout_record_restored(env: &Env, admin: &Address, scout_id: u64) {
    env.events().publish(
        (Symbol::new(env, "scout_record_restored"), admin.clone()),
        scout_id,
    );
}
