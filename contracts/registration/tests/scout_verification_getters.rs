//! Tests for `get_scout_verification` and `get_scouts` — issue #1701.
//! Also covers `unverify_scout` — issue #1879.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient, ScoutChainError};
use soroban_sdk::testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, String, Symbol, Vec};

fn setup() -> (Env, Address, RegistrationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RegistrationContract, (admin.clone(),));
    let client = RegistrationContractClient::new(&env, &contract_id);
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

#[test]
fn test_unverify_scout_clears_verified_and_sets_revocation() {
    let (env, admin, client) = setup();
    let id = register_scout(&env, &client);
    env.ledger().set_timestamp(12_345);
    client.verify_scout(&id);
    assert!(client.is_scout_verified(&id));

    env.ledger().set_timestamp(67_890);
    client.unverify_scout(&id);

    assert!(!client.is_scout_verified(&id));
    let rec = client.get_scout_verification(&id);
    assert!(!rec.verified);
    assert_eq!(rec.verified_by, Some(admin));
    assert_eq!(rec.verified_at, Some(12_345));
    assert_eq!(rec.revoked_at, Some(67_890));
    assert_eq!(
        rec.revoked_reason,
        Some(String::from_str(&env, "admin_unverify"))
    );
}

#[test]
fn test_unverify_scout_already_unverified_returns_invalid_input() {
    let (env, _, client) = setup();
    let id = register_scout(&env, &client);
    assert_eq!(
        client.try_unverify_scout(&id),
        Err(Ok(ScoutChainError::InvalidInput))
    );
}

#[test]
fn test_unverify_scout_unknown_id_returns_scout_not_found() {
    let (_, _, client) = setup();
    assert_eq!(
        client.try_unverify_scout(&999),
        Err(Ok(ScoutChainError::ScoutNotFound))
    );
}

#[test]
fn test_unverify_scout_non_admin_fails_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(RegistrationContract, (admin.clone(),));
    let client = RegistrationContractClient::new(&env, &contract_id);

    let wallet = Address::generate(&env);
    let id = client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "register_scout",
                args: (wallet.clone(), String::from_str(&env, "EU")).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .register_scout(&wallet, &String::from_str(&env, "EU"));

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "verify_scout",
                args: (id,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .verify_scout(&id);

    let non_admin = Address::generate(&env);
    let result = client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unverify_scout",
                args: (id,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_unverify_scout(&id);
    assert!(result.is_err());
    assert!(client.is_scout_verified(&id));
}

#[test]
fn test_unverify_scout_emits_event() {
    let (env, _, client) = setup();
    let id = register_scout(&env, &client);
    client.verify_scout(&id);
    client.unverify_scout(&id);

    let contract_id = client.address.clone();
    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        let (emitter, topics, data) = event;
        if emitter != contract_id {
            continue;
        }
        if topics.len() < 2 {
            continue;
        }
        let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
        if topic0 != Symbol::new(&env, "scout_unverified") {
            continue;
        }
        let topic1: u64 = topics.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, id);
        let data_id: u64 = data.into_val(&env);
        assert_eq!(data_id, id);
        found = true;
    }
    assert!(found, "ScoutUnverified event not emitted");
}
