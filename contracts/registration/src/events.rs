#![allow(deprecated)]
use soroban_sdk::{Address, Env, Symbol};

pub fn player_registered(env: &Env, player_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "player_registered"), wallet.clone()),
        player_id,
    );
}

pub fn scout_registered(env: &Env, scout_id: u64, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "scout_registered"), wallet.clone()),
        scout_id,
    );
}

pub fn profile_updated(env: &Env, player_id: u64) {
    env.events()
        .publish((Symbol::new(env, "profile_updated"),), player_id);
}

pub fn player_deregistered(env: &Env, player_id: u64) {
    env.events()
        .publish((Symbol::new(env, "player_deregistered"),), player_id);
}

pub fn player_level_synced(env: &Env, player_id: u64) {
    env.events()
        .publish((Symbol::new(env, "player_level_synced"),), player_id);
}

pub fn scout_verified(env: &Env, scout_id: u64) {
    env.events()
        .publish((Symbol::new(env, "scout_verified"),), scout_id);
}
