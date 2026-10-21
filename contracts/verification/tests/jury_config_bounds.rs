//! Bounds validation for `set_jury_config` (issue #1394).

use scoutchain_verification::{
    VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

const DAY: u64 = 86_400;
const MAX_WINDOW: u64 = 2_592_000; // 30 days

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    client.initialize(&Address::generate(&env));
    (env, client)
}

fn register_n(env: &Env, client: &VerificationContractClient, n: u32) {
    for i in 0..n {
        let w = Address::generate(env);
        client.register_validator(
            &w,
            &String::from_str(env, "UEFA-B-License-2026"),
            &String::from_str(env, "Academy"),
            &Vec::new(env),
        );
        let _ = i;
    }
}

#[test]
fn quorum_zero_rejected() {
    let (env, client) = setup();
    register_n(&env, &client, 1);
    let result = client.try_set_jury_config(&100u32, &0u32, &DAY);
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
}

#[test]
fn quorum_above_active_count_rejected() {
    let (env, client) = setup();
    register_n(&env, &client, 2);
    let result = client.try_set_jury_config(&100u32, &3u32, &DAY);
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
}

#[test]
fn voting_window_zero_rejected() {
    let (env, client) = setup();
    register_n(&env, &client, 3);
    let result = client.try_set_jury_config(&100u32, &3u32, &0u64);
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
}

#[test]
fn voting_window_below_min_rejected() {
    let (env, client) = setup();
    register_n(&env, &client, 3);
    let result = client.try_set_jury_config(&100u32, &3u32, &(DAY - 1));
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
}

#[test]
fn voting_window_above_max_rejected() {
    let (env, client) = setup();
    register_n(&env, &client, 3);
    let result = client.try_set_jury_config(&100u32, &3u32, &(MAX_WINDOW + 1));
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
}

#[test]
fn boundary_min_and_max_window_accepted() {
    let (env, client) = setup();
    register_n(&env, &client, 3);
    client.set_jury_config(&100u32, &1u32, &DAY);
    client.set_jury_config(&100u32, &3u32, &MAX_WINDOW);
    let cfg = client.get_jury_config();
    assert_eq!(cfg.voting_window_secs, MAX_WINDOW);
    assert_eq!(cfg.quorum, 3);
}
