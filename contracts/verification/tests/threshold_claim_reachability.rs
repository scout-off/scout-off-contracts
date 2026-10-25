//! Issue #1613: open k-of-n claims whose threshold becomes unreachable after
//! a revocation are aborted deterministically instead of blocking the
//! evidence for the whole voting window.

use scoutchain_shared_types::testutils::count_events;
use scoutchain_verification::{
    AttestationStatus, RevocationSeverity, VerificationContract, VerificationContractClient,
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

fn attest(
    env: &Env,
    client: &VerificationContractClient,
    v: &Address,
    player_id: u64,
    description: &String,
    evidence: &String,
) -> AttestationStatus {
    env.mock_auths(&[MockAuth {
        address: v,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest_milestone",
            args: (v.clone(), player_id, description.clone(), evidence.clone()).into_val(env),
            sub_invokes: &[],
        },
    }]);
    client.attest_milestone(v, &player_id, description, evidence)
}

#[test]
fn revocation_making_threshold_unreachable_aborts_open_claim() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    let v3 = register(&env, &client);
    client.set_milestone_threshold(&3u32);

    let player_id = 1u64;
    let description = String::from_str(&env, "hat-trick in regional final");
    let evidence = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");

    attest(&env, &client, &v1, player_id, &description, &evidence);
    attest(&env, &client, &v2, player_id, &description, &evidence);
    assert!(client.is_pending_claim_reachable(&player_id, &evidence));

    // v3 is revoked: 2 active validators < threshold 3.
    env.mock_all_auths();
    client.revoke_validator(&v3, &RevocationSeverity::Routine, &None);

    assert!(
        client.get_pending_claim(&player_id, &evidence).is_none(),
        "unreachable round is aborted and its claim removed"
    );
    assert!(!client.is_pending_claim_reachable(&player_id, &evidence));
    assert!(!client.has_attested(&player_id, &evidence, &v1));
    assert_eq!(
        count_events(&env, "attestation_round_aborted", Some(&client.address)),
        1
    );
}

#[test]
fn claim_stays_open_and_reachable_when_threshold_still_met() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let _v2 = register(&env, &client);
    let v3 = register(&env, &client);
    let _v4 = register(&env, &client);
    client.set_milestone_threshold(&3u32);

    let player_id = 2u64;
    let description = String::from_str(&env, "identity confirmed");
    let evidence = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqC");

    attest(&env, &client, &v1, player_id, &description, &evidence);

    // 4 -> 3 active, threshold 3 still reachable.
    env.mock_all_auths();
    client.revoke_validator(&v3, &RevocationSeverity::Routine, &None);

    let claim = client.get_pending_claim(&player_id, &evidence).unwrap();
    assert_eq!(claim.vote_count, 1);
    assert!(client.is_pending_claim_reachable(&player_id, &evidence));
    assert_eq!(
        count_events(&env, "attestation_round_aborted", Some(&client.address)),
        0
    );
}

#[test]
fn aborted_evidence_can_be_reattested_after_threshold_is_lowered() {
    let (env, client) = setup();
    let v1 = register(&env, &client);
    let v2 = register(&env, &client);
    let v3 = register(&env, &client);
    client.set_milestone_threshold(&3u32);

    let player_id = 3u64;
    let description = String::from_str(&env, "top speed 32km/h");
    let evidence = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqD");

    attest(&env, &client, &v1, player_id, &description, &evidence);

    env.mock_all_auths();
    client.revoke_validator(&v3, &RevocationSeverity::Routine, &None);
    client.set_milestone_threshold(&2u32);

    // The evidence is not stuck: a fresh round starts at the new threshold.
    assert_eq!(
        attest(&env, &client, &v1, player_id, &description, &evidence),
        AttestationStatus::Pending(1)
    );
    assert!(matches!(
        attest(&env, &client, &v2, player_id, &description, &evidence),
        AttestationStatus::Committed(_)
    ));
}
