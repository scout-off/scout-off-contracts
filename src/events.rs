use soroban_sdk::{Address, Env, String, Symbol, Vec};

// ── Existing event topics ──

pub fn player_registered(env: &Env, player_id: u64, wallet: Address) {
    let topics = (Symbol::new(env, "player_registered"), wallet);
    env.events().publish(topics, player_id);
}

pub fn milestone_approved(
    env: &Env,
    player_id: u64,
    validator: Address,
    milestone: Symbol,
) {
    let topics = (Symbol::new(env, "milestone_approved"), player_id);
    env.events().publish(topics, (validator, milestone));
}

pub fn progress_updated(env: &Env, player_id: u64, new_level: u32) {
    let topics = (Symbol::new(env, "progress_updated"), player_id);
    env.events().publish(topics, new_level);
}

pub fn scout_subscribed(env: &Env, scout: Address, tier: Symbol) {
    let topics = (Symbol::new(env, "scout_subscribed"), scout);
    env.events().publish(topics, tier);
}

pub fn player_contacted(env: &Env, scout: Address, player_id: u64) {
    let topics = (Symbol::new(env, "player_contacted"), scout);
    env.events().publish(topics, player_id);
}

pub fn trial_offer_logged(env: &Env, player_id: u64, scout: Address) {
    let topics = (Symbol::new(env, "trial_offer_logged"), player_id);
    env.events().publish(topics, scout);
}

pub fn fees_withdrawn(env: &Env, admin: Address, amount: i128) {
    let topics = (Symbol::new(env, "fees_withdrawn"), admin);
    env.events().publish(topics, amount);
}

// ── Updated validator_registered event (issue #39) ──

/**
 * Emit `validator_registered` event with wallet and credentials.
 *
 * Event data: (wallet: Address, credentials: String)
 *
 * # Change (issue #39)
 * Previously emitted only the wallet address. Now includes the credential
 * label so the backend indexer can populate the validators table without
 * a separate `get_validator` call.
 */
pub fn validator_registered(env: &Env, wallet: Address, credentials: String) {
    let topics = (Symbol::new(env, "validator_registered"), wallet);
    // Event data now includes both wallet and credentials
    env.events().publish(topics, (wallet, credentials));
}

pub fn validator_revoked(env: &Env, wallet: Address) {
    let topics = (Symbol::new(env, "validator_revoked"), wallet);
    env.events().publish(topics, wallet);
}