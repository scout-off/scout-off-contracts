use soroban_sdk::{Address, Env, Symbol};
use crate::types::ProgressLevel;

pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "admin_transferred"),),
        (old_admin.clone(), new_admin.clone()),
    );
}

pub fn progress_updated(
    env: &Env,
    player_id: u64,
    new_level: &ProgressLevel,
    updated_by: &Address,
) {
    env.events().publish(
        (
            Symbol::new(env, "progress_updated"),
            updated_by.clone(),
        ),
        (player_id, new_level.clone()),
    );
}
