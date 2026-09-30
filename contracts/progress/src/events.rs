#![allow(deprecated, dead_code)]
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{Address, Env, Symbol};

pub const ADMIN_TRANSFERRED: &str = "admin_transferred";
pub const ADMIN_TRANSFER_PROPOSED: &str = "admin_transfer_proposed";
pub const PROGRESS_UPDATED: &str = "progress_updated";
pub const PLAYER_LEVEL_RESET: &str = "player_level_reset";
pub const WIRING_UPDATED: &str = "wiring_updated";

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

/// topics: (event_name, updated_by)  data: (player_id, old_level, new_level)
pub fn progress_updated(
    env: &Env,
    player_id: u64,
    old_level: &ProgressLevel,
    new_level: &ProgressLevel,
    updated_by: &Address,
    _milestone_ref: u32,
) {
    env.events().publish(
        (Symbol::new(env, "progress_updated"), updated_by.clone()),
        (player_id, old_level.clone(), new_level.clone()),
    );
}

/// topics: (event_name, admin)  data: (player_id, old_level, target_level)
pub fn player_level_reset(
    env: &Env,
    admin: &Address,
    player_id: u64,
    old_level: &ProgressLevel,
    target_level: &ProgressLevel,
) {
    env.events().publish(
        (Symbol::new(env, "player_level_reset"), admin.clone()),
        (player_id, old_level.clone(), target_level.clone()),
    );
}

/// topics: (event_name, admin, link)  data: (new_address, new_epoch)
///
/// Emitted by every `set_registration_contract` / `set_verification_contract`
/// / `set_scout_access_contract` call. `link` identifies which of this
/// contract's three peer pointers changed (`"registration_contract"`,
/// `"verification_contract"`, or `"scout_access_contract"`) so a single
/// indexer subscription can distinguish them without three separate event
/// names. See `docs/WIRING_REGISTRY_DESIGN.md`.
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

/// Emitted by `restore_player_level_record` when an admin re-extends an
/// archived or expired player-level entry's TTL back to the policy value.
/// topics: (event_name, admin)  data: player_id
pub fn player_level_record_restored(env: &Env, admin: &Address, player_id: u64) {
    env.events().publish(
        (
            Symbol::new(env, "player_level_record_restored"),
            admin.clone(),
        ),
        player_id,
    );
}
