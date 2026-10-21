//! Milestone threshold vs active validator count (issue #1395).

use scoutchain_verification::{
    RevocationSeverity, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    client.initialize(&Address::generate(&env));
    (env, client)
}

fn register(env: &Env, client: &VerificationContractClient) -> Address {
    let w = Address::generate(env);
    client.register_validator(
        &w,
        &String::from_str(env, "UEFA-B-License-2026"),
        &String::from_str(env, "Academy"),
        &Vec::new(env),
    );
    w
}

#[test]
fn cannot_set_threshold_above_active_count() {
    let (env, client) = setup();
    let _v1 = register(&env, &client);
    let _v2 = register(&env, &client);
    // 2 active — threshold 3 must fail.
    let result = client.try_set_milestone_threshold(&3u32);
    assert_eq!(
        result,
        Err(Ok(VerificationError::ThresholdExceedsActiveValidators))
    );
}

#[test]
fn can_set_threshold_equal_to_active_count() {
    let (env, client) = setup();
    let _v1 = register(&env, &client);
    let _v2 = register(&env, &client);
    let _v3 = register(&env, &client);
    client.set_milestone_threshold(&3u32);
    assert_eq!(client.get_milestone_threshold(), 3);

    let status = client.get_milestone_threshold_status();
    assert_eq!(status.threshold, 3);
    assert_eq!(status.active_validator_count, 3);
    assert!(status.reachable);
}

#[test]
fn revocation_crossing_below_threshold_marks_unreachable() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let _v2 = register(&env, &client);
    let _v3 = register(&env, &client);
    client.set_milestone_threshold(&3u32);

    assert!(client.get_milestone_threshold_status().reachable);

    client.revoke_validator(&v1, &RevocationSeverity::Routine, &None);

    let status = client.get_milestone_threshold_status();
    assert_eq!(status.active_validator_count, 2);
    assert_eq!(status.threshold, 3);
    assert!(
        !status.reachable,
        "revocation that drops active below threshold must surface unreachable"
    );
}
