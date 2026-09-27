#![allow(deprecated, dead_code)]
use soroban_sdk::{Address, BytesN, Env, Symbol};

pub const PLAYER_REGISTERED: &str = "player_registered";
pub const SCOUT_REGISTERED: &str = "scout_registered";
pub const PROFILE_UPDATED: &str = "profile_updated";
pub const PLAYER_DEREGISTERED: &str = "player_deregistered";
pub const PLAYER_DEACTIVATED: &str = "player_deactivated";
pub const PLAYER_REACTIVATED: &str = "player_reactivated";
pub const PLAYER_LEVEL_SYNCED: &str = "player_level_synced";
pub const SCOUT_VERIFIED: &str = "scout_verified";
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

/// topics: (event_name, wallet)  data: player_id
pub fn player_registered(env: &Env, player_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_registered"), wallet.clone()),
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

/// Emitted before `update_current_contract_wasm` — attributed to the old code version.
/// topics: (event_name, admin)  data: new_wasm_hash
pub fn contract_upgraded(env: &Env, admin: &Address, new_wasm_hash: &BytesN<32>) {
    env.events().publish(
        (Symbol::new(env, CONTRACT_UPGRADED), admin.clone()),
        new_wasm_hash.clone(),
    );
}
