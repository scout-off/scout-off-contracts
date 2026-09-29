//! Adversarial atomicity tests for `registration.register_player` — Issue #1057.
//!
//! `register_player` performs several persistent writes (profile, wallet->id
//! index, level, player index, composite/level indexes, cooldown). Soroban
//! commits all of them atomically: a rejected registration must leave NO
//! partial state behind. These tests prove that a failed registration neither
//! duplicates an existing player nor leaks a half-written profile.

use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

struct Harness {
    env: Env,
    client: RegistrationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register_contract(None, RegistrationContract);
    let client = RegistrationContractClient::new(&env, &id);
    client.initialize(&admin);
    Harness { env, client }
}

fn valid_vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Forward"),
        region: String::from_str(env, "EU"),
        nationality: String::from_str(env, "FR"),
    }
}

fn one_hash(env: &Env) -> Vec<String> {
    let mut v = Vec::new(env);
    v.push_back(String::from_str(env, "bafytestcid"));
    v
}

/// A duplicate registration must be rejected and must not create a second
/// player or leave any partial state.
#[test]
fn test_duplicate_registration_is_atomic() {
    let h = setup();
    let wallet = Address::generate(&h.env);

    let ok = h
        .client
        .try_register_player(&wallet, &valid_vitals(&h.env), &one_hash(&h.env));
    assert!(ok.is_ok(), "first registration must succeed: {ok:?}");
    assert_eq!(h.client.get_player_count(), 1);

    let dup = h
        .client
        .try_register_player(&wallet, &valid_vitals(&h.env), &one_hash(&h.env));
    assert!(
        dup.is_err(),
        "duplicate registration must be rejected: {dup:?}"
    );

    // No second player was created, and the original wallet mapping is intact.
    assert_eq!(
        h.client.get_player_count(),
        1,
        "no second player may be created"
    );
    assert!(
        h.client.try_get_player_by_wallet(&wallet).is_ok(),
        "original wallet mapping must remain"
    );
}

/// An invalid registration (age 0) must be rejected and must commit no state.
#[test]
fn test_invalid_registration_commits_no_state() {
    let h = setup();
    let wallet = Address::generate(&h.env);

    let bad = PlayerVitals {
        age: 0,
        position: String::from_str(&h.env, "Forward"),
        region: String::from_str(&h.env, "EU"),
        nationality: String::from_str(&h.env, "FR"),
    };
    let res = h
        .client
        .try_register_player(&wallet, &bad, &one_hash(&h.env));
    assert!(
        res.is_err(),
        "invalid registration must be rejected: {res:?}"
    );

    assert_eq!(h.client.get_player_count(), 0, "no player may be created");
    assert!(
        h.client.try_get_player_by_wallet(&wallet).is_err(),
        "no partial profile may be written"
    );
}
