//! Issue #1398: prune expired attestation claims and clean vote keys.

use scoutchain_verification::{
    AttestationStatus, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, String,
};

const WINDOW: u64 = 1_209_600;

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

#[test]
fn prune_expired_claim_removes_vote_keys() {
    let (env, client) = setup();
    client.set_milestone_threshold(&3u32);
    let v1 = register(&env, &client);
    let player = 9u64;
    let desc = String::from_str(&env, "pending claim");
    let evidence = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");

    env.mock_auths(&[MockAuth {
        address: &v1,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest_milestone",
            args: (v1.clone(), player, desc.clone(), evidence.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        client.attest_milestone(&v1, &player, &desc, &evidence),
        AttestationStatus::Pending(1)
    );
    assert!(client.has_attested(&player, &evidence, &v1));
    assert!(client.get_pending_claim(&player, &evidence).is_some());

    // Not expired yet.
    assert_eq!(
        client.try_prune_expired_claim(&player, &evidence),
        Err(Ok(VerificationError::ClaimNotExpired))
    );

    env.ledger().with_mut(|l| {
        l.timestamp += WINDOW + 1;
    });
    assert!(client.is_attestation_window_expired(&player, &evidence));

    client.prune_expired_claim(&player, &evidence);
    assert!(client.get_pending_claim(&player, &evidence).is_none());
    assert!(!client.has_attested(&player, &evidence, &v1));
    assert_eq!(
        client.try_prune_expired_claim(&player, &evidence),
        Err(Ok(VerificationError::ClaimNotFound))
    );
}
