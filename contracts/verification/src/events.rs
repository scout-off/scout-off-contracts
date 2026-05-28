use soroban_sdk::{Address, Env, Symbol};

pub fn milestone_approved(env: &Env, player_id: u64, validator: &Address) {
    env.events().publish(
        (Symbol::new(env, "milestone_approved"), validator.clone()),
        player_id,
    );
}

pub fn validator_registered(env: &Env, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "validator_registered"),),
        wallet.clone(),
    );
}

pub fn validator_revoked(env: &Env, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "validator_revoked"),),
        wallet.clone(),
    );
}

pub fn validator_credentials_updated(env: &Env, wallet: &Address) {
    env.events().publish(
        (Symbol::new(env, "validator_credentials_updated"),),
        wallet.clone(),
    );
}
