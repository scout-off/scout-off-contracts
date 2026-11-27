//! Tests for the append-only scout verification history (issue #1808).

use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String};

fn setup() -> (Env, RegistrationContractClient<'static>, u64) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(RegistrationContract, (admin,));
    let client = RegistrationContractClient::new(&env, &id);
    let scout = client.register_scout(&Address::generate(&env), &String::from_str(&env, "EU"));
    (env, client, scout)
}

#[test]
fn verify_unverify_verify_keeps_three_entries_in_order() {
    let (env, client, scout) = setup();
    env.ledger().set_timestamp(100);
    client.verify_scout(&scout);
    env.ledger().set_timestamp(200);
    client.unverify_scout(&scout, &String::from_str(&env, "bad"));
    env.ledger().set_timestamp(300);
    client.verify_scout(&scout);

    let h = client.get_scout_verification_history(&scout);
    assert_eq!(h.len(), 3);
    assert!(h.get(0).unwrap().verified);
    assert_eq!(h.get(0).unwrap().verified_at, Some(100));
    assert!(!h.get(1).unwrap().verified);
    assert_eq!(h.get(1).unwrap().revoked_at, Some(200));
    assert_eq!(
        h.get(1).unwrap().revoked_reason,
        Some(String::from_str(&env, "bad"))
    );
    assert!(h.get(2).unwrap().verified);
    assert_eq!(h.get(2).unwrap().verified_at, Some(300));

    let cur = client.get_scout_verification(&scout);
    assert!(cur.verified);
    assert_eq!(cur.verified_at, Some(300));
}

#[test]
fn history_is_bounded_to_ten_entries() {
    let (env, client, scout) = setup();
    for i in 0..8u64 {
        env.ledger().set_timestamp(1000 + i);
        client.verify_scout(&scout);
        client.unverify_scout(&scout, &String::from_str(&env, "r"));
    }
    let h = client.get_scout_verification_history(&scout);
    assert_eq!(h.len(), 10);
    // Oldest 6 entries dropped: first retained is the verify of iteration 3.
    assert_eq!(h.get(0).unwrap().verified_at, Some(1003));
}

#[test]
fn history_unknown_scout_errors() {
    let (_env, client, _) = setup();
    assert!(client.try_get_scout_verification_history(&999).is_err());
}
