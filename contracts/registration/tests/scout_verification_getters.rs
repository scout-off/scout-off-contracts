//! Tests for `get_scout_verification` and `get_scouts` — issue #1701.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient, ScoutChainError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String, Vec};

fn setup() -> (Env, Address, RegistrationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    (env, admin, client)
}

fn register_scout(env: &Env, client: &RegistrationContractClient) -> u64 {
    let wallet = Address::generate(env);
    client.register_scout(&wallet, &String::from_str(env, "EU"))
}

#[test]
fn test_new_scout_verification_is_unverified_with_empty_fields() {
    let (env, _, client) = setup();
    let id = register_scout(&env, &client);
    let rec = client.get_scout_verification(&id);
    assert!(!rec.verified);
    assert_eq!(rec.verified_by, None);
    assert_eq!(rec.verified_at, None);
    assert_eq!(rec.evidence_ref, None);
    assert_eq!(rec.method, None);
}

#[test]
fn test_verify_scout_populates_verification_record() {
    let (env, admin, client) = setup();
    let id = register_scout(&env, &client);
    env.ledger().set_timestamp(12_345);
    client.verify_scout(&id);
    let rec = client.get_scout_verification(&id);
    assert!(rec.verified);
    assert_eq!(rec.verified_by, Some(admin));
    assert_eq!(rec.verified_at, Some(12_345));
    assert_eq!(rec.method, Some(String::from_str(&env, "admin_manual")));
}

#[test]
fn test_get_scout_verification_unknown_id_returns_scout_not_found() {
    let (_, _, client) = setup();
    assert_eq!(
        client.try_get_scout_verification(&999),
        Err(Ok(ScoutChainError::ScoutNotFound))
    );
}

#[test]
fn test_get_scouts_with_21_ids_returns_invalid_input() {
    let (env, _, client) = setup();
    let mut ids = Vec::new(&env);
    for i in 0..21u64 {
        ids.push_back(i);
    }
    assert_eq!(
        client.try_get_scouts(&ids),
        Err(Ok(ScoutChainError::InvalidInput))
    );
}

#[test]
fn test_get_scouts_mixed_ids_returns_only_hits_in_input_order() {
    let (env, _, client) = setup();
    let a = register_scout(&env, &client);
    let b = register_scout(&env, &client);
    let ids = Vec::from_array(&env, [b, 999, a, 1000]);
    let profiles = client.get_scouts(&ids);
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles.get(0).unwrap().scout_id, b);
    assert_eq!(profiles.get(1).unwrap().scout_id, a);
}
