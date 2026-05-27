use soroban_sdk::{Address, Env, Symbol, symbol_short};

pub fn contract_initialized(env: &Env, admin: &Address) {
    env.events().publish(
        (Symbol::new(env, "contract_initialized"),),
        admin.clone(),
    );
}

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
    env.events().publish(
        (symbol_short!("prof_upd"),),
        player_id,
    );
}
