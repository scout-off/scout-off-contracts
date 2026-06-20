#![allow(deprecated)]
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{Address, Env, Symbol};

pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "admin_transferred"),),
        (old_admin.clone(), new_admin.clone()),
    );
}

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

pub fn player_level_reset(
    env: &Env,
    player_id: u64,
    old_level: &ProgressLevel,
    target_level: &ProgressLevel,
) {
    env.events().publish(
        (Symbol::new(env, "player_level_reset"),),
        (player_id, old_level.clone(), target_level.clone()),
    );
}
