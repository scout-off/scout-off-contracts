//! Issue #1397: subsequent attest_milestone votes must match the round's
//! description hash or be rejected with DescriptionMismatch.

use scoutchain_verification::{
    AttestationStatus, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, String,
};

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    client.initialize(&Address::generate(&env));
    (env, client)
}

fn register(env: &Env, client: &VerificationContractClient) -> Address {
    let w = Address::generate(env);
    client.register_validator(
        &w,
        &String::from_str(env, "UEFA-B-License-2026"),
        &String::from_str(env, "Academy"),
        &soroban_sdk::Vec::new(env),
    );
    w
}

fn cid(env: &Env) -> String {
    String::from_str(env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB")
}

fn attest(
    env: &Env,
    client: &VerificationContractClient,
    v: &Address,
    player: u64,
    desc: &String,
    evidence: &String,
) -> AttestationStatus {
    env.mock_auths(&[MockAuth {
        address: v,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest_milestone",
            args: (v.clone(), player, desc.clone(), evidence.clone()).into_val(env),
            sub_invokes: &[],
        },
    }]);
    client.attest_milestone(v, &player, desc, evidence)
}

#[test]
fn mismatched_description_is_rejected() {
    let (env, client) = setup();
    client.set_milestone_threshold(&3u32);
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    let player = 1u64;
    let evidence = cid(&env);
    let d1 = String::from_str(&env, "U17 national team selection");
    let d2 = String::from_str(&env, "completely different narrative");

    assert_eq!(
        attest(&env, &client, &v1, player, &d1, &evidence),
        AttestationStatus::Pending(1)
    );

    env.mock_auths(&[MockAuth {
        address: &v2,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest_milestone",
            args: (v2.clone(), player, d2.clone(), evidence.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let result = client.try_attest_milestone(&v2, &player, &d2, &evidence);
    assert_eq!(result, Err(Ok(VerificationError::DescriptionMismatch)));

    let claim = client.get_pending_claim(&player, &evidence).unwrap();
    assert_eq!(claim.vote_count, 1);
    assert_eq!(claim.description, d1);
}

#[test]
fn matching_description_still_commits() {
    let (env, client) = setup();
    client.set_milestone_threshold(&2u32);
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    let player = 2u64;
    let evidence = cid(&env);
    let desc = String::from_str(&env, "hat-trick in cup final");

    assert_eq!(
        attest(&env, &client, &v1, player, &desc, &evidence),
        AttestationStatus::Pending(1)
    );
    match attest(&env, &client, &v2, player, &desc, &evidence) {
        AttestationStatus::Committed(1) => {}
        other => panic!("expected commit, got {other:?}"),
    }
}
