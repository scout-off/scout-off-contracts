use soroban_sdk::{testutils::Events, vec, Env, IntoVal, String, Symbol};

use crate::events::validator_registered;

#[test]
fn test_validator_registered_emits_wallet_and_credentials() {
    let env = Env::default();
    let wallet = env.register_contract(None, crate::MockContract);
    let credentials = String::from_str(&env, "UEFA Pro License — Coach Level A");

    validator_registered(&env, wallet.clone(), credentials.clone());

    // Retrieve all emitted events
    let events = env.events().all();

    // There should be exactly one event
    assert_eq!(events.len(), 1);

    let event = events.get(0).unwrap();

    // Verify event topics
    let expected_topics = vec![
        &env,
        Symbol::new(&env, "validator_registered").into_val(&env),
        wallet.clone().into_val(&env),
    ];
    assert_eq!(event.0, expected_topics);

    // Verify event data includes both wallet and credentials
    let expected_data = (wallet.clone(), credentials.clone()).into_val(&env);
    assert_eq!(event.1, expected_data);
}

#[test]
fn test_validator_registered_credentials_persisted_correctly() {
    let env = Env::default();
    let wallet = env.register_contract(None, crate::MockContract);
    let credentials = String::from_str(&env, "FIFA Certified Academy Director");

    validator_registered(&env, wallet.clone(), credentials.clone());

    let events = env.events().all();
    let event = events.get(0).unwrap();

    // Decode the tuple from event data
    let data: (soroban_sdk::Address, soroban_sdk::String) =
        event.1.into_val(&env);

    assert_eq!(data.0, wallet);
    assert_eq!(data.1, credentials);
}

#[test]
fn test_validator_registered_with_empty_credentials() {
    let env = Env::default();
    let wallet = env.register_contract(None, crate::MockContract);
    let credentials = String::from_str(&env, "");

    validator_registered(&env, wallet.clone(), credentials.clone());

    let events = env.events().all();
    let event = events.get(0).unwrap();

    let data: (soroban_sdk::Address, soroban_sdk::String) =
        event.1.into_val(&env);

    assert_eq!(data.0, wallet);
    assert_eq!(data.1, String::from_str(&env, ""));
}

#[test]
fn test_validator_registered_with_long_credentials() {
    let env = Env::default();
    let wallet = env.register_contract(None, crate::MockContract);
    let long_cred = String::from_str(
        &env,
        "UEFA Pro License — Coach Level A — Specialization: Youth Development — \
         Certified: 2024-01-15 — Institution: Royal Spanish Football Federation — \
         License ID: ESP-2024-001337",
    );

    validator_registered(&env, wallet.clone(), long_cred.clone());

    let events = env.events().all();
    let event = events.get(0).unwrap();

    let data: (soroban_sdk::Address, soroban_sdk::String) =
        event.1.into_val(&env);

    assert_eq!(data.0, wallet);
    assert_eq!(data.1, long_cred);
}