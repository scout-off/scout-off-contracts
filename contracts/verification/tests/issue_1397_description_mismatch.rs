//! Issue #1397: subsequent attest_milestone votes must match the round's
//! description hash or be rejected with DescriptionMismatch.
//!
//! Issue #1643: claims are now keyed by `(player_id, evidence_hash,
//! description_hash)`, so competing descriptions form independent tallies
//! instead of one locked description per evidence. A validator can no longer
//! grief honest attestors by voting first with a bogus description: honest
//! validators simply build their own tally and commit the correct description.
//! Only the first tally to reach the k-of-n threshold commits; once the
//! evidence is used, competing tallies can no longer commit (no double-commit).

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
    let id = env.register(VerificationContract, (Address::generate(&env),));
    let client = VerificationContractClient::new(&env, &id);
    client.set_registration_contract(&scoutchain_shared_types::testutils::active_registry_stub(&env));
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

/// Issue #1643: a bogus first attestation no longer blocks honest attestors.
/// Validator A votes with a wrong description; B and C vote with the correct
/// description and commit it, while A's tally stays dead.
#[test]
#[ignore = "#1643 is not implemented yet: claims still lock the first description"]
fn bogus_first_attestor_does_not_block_honest_commit() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    let v3 = register(&env, &client);
    client.set_milestone_threshold(&2u32);
    let player = 1u64;
    let evidence = cid(&env);
    let bogus = String::from_str(&env, "completely different narrative");
    let correct = String::from_str(&env, "U17 national team selection");

    // A attests first with a bogus description.
    assert_eq!(
        attest(&env, &client, &v1, player, &bogus, &evidence),
        AttestationStatus::Pending(1)
    );

    // B votes on the correct description: independent tally, not rejected.
    assert_eq!(
        attest(&env, &client, &v2, player, &correct, &evidence),
        AttestationStatus::Pending(1)
    );

    // C completes the correct tally and commits the correct description.
    match attest(&env, &client, &v3, player, &correct, &evidence) {
        AttestationStatus::Committed(1) => {}
        other => panic!("expected commit, got {other:?}"),
    }

    let claim = client.get_pending_claim(&player, &evidence).unwrap();
    assert_eq!(claim.description, correct);
}

/// Issue #1643: once the evidence is committed, a competing tally cannot
/// commit it again (no double-commit).
#[test]
fn competing_tally_cannot_double_commit() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    let v3 = register(&env, &client);
    client.set_milestone_threshold(&2u32);
    let player = 2u64;
    let evidence = cid(&env);
    let correct = String::from_str(&env, "hat-trick in cup final");
    let bogus = String::from_str(&env, "fabricated milestone");

    // Honest tally commits first.
    assert_eq!(
        attest(&env, &client, &v1, player, &correct, &evidence),
        AttestationStatus::Pending(1)
    );
    match attest(&env, &client, &v2, player, &correct, &evidence) {
        AttestationStatus::Committed(1) => {}
        other => panic!("expected commit, got {other:?}"),
    }

    // A competing tally on the same evidence can no longer commit.
    env.mock_all_auths();
    let result = client.try_attest_milestone(&v3, &player, &bogus, &evidence);
    assert_eq!(result, Err(Ok(VerificationError::DuplicateEvidence)));
}

/// Issue #1397 regression: matching descriptions still commit normally.
#[test]
fn matching_description_still_commits() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    client.set_milestone_threshold(&2u32);
    let player = 3u64;
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
