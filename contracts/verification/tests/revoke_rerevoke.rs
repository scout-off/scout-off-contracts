//! Re-revocation transition table (issue #1393).
//!
//! | Current → Requested | Expected |
//! |---|---|
//! | Routine → Routine | ValidatorAlreadyRevoked |
//! | ForCause → ForCause | ValidatorAlreadyRevoked |
//! | ForCause → Routine | ValidatorAlreadyRevoked (downgrade) |
//! | Routine → ForCause | allowed; preserves original revoked_at; starts cascade |

use scoutchain_verification::{
    RevocationSeverity, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn register(env: &Env, client: &VerificationContractClient) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, "UEFA-B-License-2026"),
        &String::from_str(env, "Default Academy"),
        &Vec::new(env),
    );
    wallet
}

#[test]
fn routine_then_routine_is_rejected() {
    let (env, client) = setup();
    let v = register(&env, &client);
    client.revoke_validator(&v, &RevocationSeverity::Routine, &None);
    let result = client.try_revoke_validator(&v, &RevocationSeverity::Routine, &None);
    assert_eq!(result, Err(Ok(VerificationError::ValidatorAlreadyRevoked)));
}

#[test]
fn for_cause_then_for_cause_is_rejected() {
    let (env, client) = setup();
    let v = register(&env, &client);
    client.revoke_validator(&v, &RevocationSeverity::ForCause, &None);
    let result = client.try_revoke_validator(&v, &RevocationSeverity::ForCause, &None);
    assert_eq!(result, Err(Ok(VerificationError::ValidatorAlreadyRevoked)));
}

#[test]
fn for_cause_then_routine_downgrade_is_rejected() {
    let (env, client) = setup();
    let v = register(&env, &client);
    client.revoke_validator(&v, &RevocationSeverity::ForCause, &None);
    let result = client.try_revoke_validator(&v, &RevocationSeverity::Routine, &None);
    assert_eq!(result, Err(Ok(VerificationError::ValidatorAlreadyRevoked)));
}

#[test]
fn routine_then_for_cause_escalation_preserves_original_timestamp() {
    let (env, client) = setup();
    let v = register(&env, &client);

    client.revoke_validator(
        &v,
        &RevocationSeverity::Routine,
        &Some(String::from_str(&env, "scheduled offboarding")),
    );
    let first = client
        .get_revocation_record(&v)
        .expect("routine record must exist");
    assert_eq!(first.severity, RevocationSeverity::Routine);
    assert_eq!(first.revoked_at, 1_000_000);

    env.ledger().with_mut(|l| l.timestamp = 1_000_500);

    client.revoke_validator(
        &v,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "credential fraud discovered")),
    );
    let escalated = client
        .get_revocation_record(&v)
        .expect("escalated record must exist");
    assert_eq!(escalated.severity, RevocationSeverity::ForCause);
    assert_eq!(
        escalated.revoked_at, first.revoked_at,
        "original revoked_at must be preserved on escalation"
    );
    assert_eq!(
        escalated.reason,
        String::from_str(&env, "credential fraud discovered")
    );

    let history = client.get_revocation_history(&v);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().severity, RevocationSeverity::Routine);
    assert_eq!(
        history.get(0).unwrap().reason,
        String::from_str(&env, "scheduled offboarding")
    );
}

#[test]
fn batch_revoke_rejects_already_revoked_same_severity() {
    let (env, client) = setup();
    let v = register(&env, &client);
    client.revoke_validator(&v, &RevocationSeverity::Routine, &None);

    let wallets = soroban_sdk::vec![&env, v.clone()];
    let result =
        client.try_batch_revoke_validators(&wallets, &RevocationSeverity::Routine, &None);
    assert_eq!(result, Err(Ok(VerificationError::ValidatorAlreadyRevoked)));
}
