//! Property-based tests for the admin propose/accept flow (#824).
//!
//! These tests were originally written against an older API and no longer
//! compiled: they imported `Address` without the `testutils::Address` trait
//! that provides `generate`, and passed an argument to `accept_admin`, which
//! takes none. The acceptor is not a parameter — `accept_admin` reads
//! `DataKey::PendingAdmin` and calls `require_auth()` on it, so "who is
//! accepting" is expressed through the authorization context, not through an
//! argument.
//!
//! That distinction matters: under a blanket `env.mock_all_auths()` every
//! `require_auth()` succeeds, so a test that merely called `accept_admin` could
//! never show that the *wrong* address is refused. Each negative test below
//! therefore scopes authorization to a single address with `mock_auths` so the
//! rejection is genuinely proven.

use scoutchain_verification::{
    DataKey, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

struct Harness {
    env: Env,
    admin: Address,
    client: VerificationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    Harness { env, admin, client }
}

/// Authorize exactly one `accept_admin` invocation, by `who`, and nothing else.
/// Any `require_auth()` on a different address will fail.
fn current_admin(h: &Harness) -> Address {
    h.env.as_contract(&h.client.address, || {
        h.env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("admin must be set after initialize")
    })
}

fn accept_admin_as(h: &Harness, who: &Address) {
    h.env.mock_auths(&[MockAuth {
        address: who,
        invoke: &MockAuthInvoke {
            contract: &h.client.address,
            fn_name: "accept_admin",
            args: ().into_val(&h.env),
            sub_invokes: &[],
        },
    }]);
}

#[test]
fn test_only_proposed_can_accept() {
    let h = setup();
    let proposed = Address::generate(&h.env);
    h.client.propose_admin(&proposed);

    accept_admin_as(&h, &proposed);
    assert!(h.client.try_accept_admin().is_ok());

    assert_eq!(
        current_admin(&h),
        proposed,
        "admin must be the newly accepted address"
    );
}

#[test]
fn test_non_proposed_cannot_accept() {
    let h = setup();
    let proposed = Address::generate(&h.env);
    let attacker = Address::generate(&h.env);
    h.client.propose_admin(&proposed);

    // Only the attacker's signature is available; the contract demands the
    // pending admin's, so the call must fail.
    accept_admin_as(&h, &attacker);
    assert!(
        h.client.try_accept_admin().is_err(),
        "an address that was never proposed must not be able to accept"
    );

    assert_eq!(
        current_admin(&h),
        h.admin,
        "a failed acceptance must leave the admin unchanged"
    );
}

#[test]
fn test_double_propose_replaces_pending() {
    let h = setup();
    let first = Address::generate(&h.env);
    let second = Address::generate(&h.env);
    h.client.propose_admin(&first);
    h.client.propose_admin(&second);

    // The superseded proposal is no longer the pending admin.
    accept_admin_as(&h, &first);
    assert!(
        h.client.try_accept_admin().is_err(),
        "a superseded proposal must not be acceptable"
    );

    accept_admin_as(&h, &second);
    assert!(
        h.client.try_accept_admin().is_ok(),
        "the most recent proposal must be acceptable"
    );
    assert_eq!(current_admin(&h), second);
}

#[test]
fn test_admin_unchanged_before_accept() {
    let h = setup();
    let new_admin = Address::generate(&h.env);
    h.client.propose_admin(&new_admin);

    // Proposing alone must not transfer authority.
    assert_eq!(
        current_admin(&h),
        h.admin,
        "propose_admin must not change the admin on its own"
    );

    let third = Address::generate(&h.env);
    h.client.propose_admin(&third);
    assert_eq!(
        current_admin(&h),
        h.admin,
        "re-proposing must still not change the admin"
    );

    accept_admin_as(&h, &third);
    assert!(h.client.try_accept_admin().is_ok());
    assert_eq!(current_admin(&h), third);
}

#[test]
fn test_replaced_proposal_cannot_accept() {
    let h = setup();
    let first = Address::generate(&h.env);
    let second = Address::generate(&h.env);
    h.client.propose_admin(&first);
    h.client.propose_admin(&second);

    accept_admin_as(&h, &first);
    assert!(
        h.client.try_accept_admin().is_err(),
        "the replaced proposal must be inert"
    );
    assert_eq!(current_admin(&h), h.admin);
}

/// Accepting with no proposal outstanding fails with `PendingAdminNotSet`
/// rather than trapping or silently succeeding.
#[test]
fn test_accept_without_proposal_is_rejected() {
    let h = setup();
    let someone = Address::generate(&h.env);

    accept_admin_as(&h, &someone);
    assert_eq!(
        h.client.try_accept_admin(),
        Err(Ok(VerificationError::PendingAdminNotSet)),
        "accepting with no pending proposal must fail cleanly"
    );
    assert_eq!(current_admin(&h), h.admin);
}
