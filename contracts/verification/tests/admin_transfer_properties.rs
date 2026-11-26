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
    Address, Env, IntoVal, Symbol,
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
    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);
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

fn pending_admin(h: &Harness) -> Option<Address> {
    h.env.as_contract(&h.client.address, || {
        h.env.storage().persistent().get(&DataKey::PendingAdmin)
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

/// Authorize exactly one `transfer_admin` invocation, by `who`, and nothing
/// else. Any `require_auth()` on a different address will fail.
fn transfer_admin_as(h: &Harness, who: &Address, new_admin: &Address) {
    h.env.mock_auths(&[MockAuth {
        address: who,
        invoke: &MockAuthInvoke {
            contract: &h.client.address,
            fn_name: "transfer_admin",
            args: (new_admin.clone(),).into_val(&h.env),
            sub_invokes: &[],
        },
    }]);
    h.client.transfer_admin(new_admin);
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

/// `transfer_admin` is a one-step-named alias for `propose_admin`: it must
/// only *propose* the new admin, leaving the current admin in place until the
/// proposal is accepted.
#[test]
fn test_transfer_admin_only_proposes() {
    let h = setup();
    let new_admin = Address::generate(&h.env);

    transfer_admin_as(&h, &h.admin, &new_admin);

    assert_eq!(
        current_admin(&h),
        h.admin,
        "transfer_admin must not change the admin on its own"
    );
    assert_eq!(
        pending_admin(&h),
        Some(new_admin),
        "transfer_admin must record the new admin as pending"
    );
}

/// After `transfer_admin(B)`, B's acceptance is still required: without B's
/// authorization `accept_admin` fails and the admin stays A; with it, the
/// admin becomes B.
#[test]
fn test_transfer_admin_requires_accept() {
    let h = setup();
    let new_admin = Address::generate(&h.env);

    transfer_admin_as(&h, &h.admin, &new_admin);
    assert_eq!(current_admin(&h), h.admin);

    // Someone other than the pending admin cannot complete the transfer.
    let attacker = Address::generate(&h.env);
    accept_admin_as(&h, &attacker);
    assert!(
        h.client.try_accept_admin().is_err(),
        "accept_admin must require the pending admin's authorization"
    );
    assert_eq!(
        current_admin(&h),
        h.admin,
        "a failed acceptance must leave the admin unchanged"
    );

    // The pending admin can.
    accept_admin_as(&h, &new_admin);
    assert!(
        h.client.try_accept_admin().is_ok(),
        "the pending admin must be able to accept"
    );
    assert_eq!(current_admin(&h), new_admin);
}

/// A non-admin caller must not be able to invoke `transfer_admin`.
#[test]
fn test_transfer_admin_non_admin_rejected() {
    let h = setup();
    let attacker = Address::generate(&h.env);
    let new_admin = Address::generate(&h.env);

    // Only the attacker's signature is available; the contract demands the
    // current admin's, so the call must fail.
    h.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &h.client.address,
            fn_name: "transfer_admin",
            args: (new_admin.clone(),).into_val(&h.env),
            sub_invokes: &[],
        },
    }]);
    assert!(
        h.client.try_transfer_admin(&new_admin).is_err(),
        "a non-admin must not be able to call transfer_admin"
    );

    assert_eq!(
        current_admin(&h),
        h.admin,
        "a rejected transfer_admin must leave the admin unchanged"
    );
    assert_eq!(
        pending_admin(&h),
        None,
        "a rejected transfer_admin must not record a pending admin"
    );
}

/// The events emitted by `transfer_admin` must match those emitted by
/// `propose_admin`, since the alias delegates to it.
#[test]
fn test_transfer_admin_emits_propose_event() {
    let via_propose = setup();
    let proposed = Address::generate(&via_propose.env);
    via_propose.client.propose_admin(&proposed);
    let propose_events = via_propose.env.events().all();

    let via_transfer = setup();
    let transferred = Address::generate(&via_transfer.env);
    transfer_admin_as(&via_transfer, &via_transfer.admin, &transferred);
    let transfer_events = via_transfer.env.events().all();

    assert_eq!(
        transfer_events, propose_events,
        "transfer_admin must emit the same events as propose_admin"
    );
}

/// Pin the contract kind symbol used by the wiring handshake (#1610).
#[test]
fn test_contract_kind() {
    let h = setup();
    assert_eq!(
        h.client.contract_kind(),
        Symbol::new(&h.env, "verification"),
        "contract_kind must return the exact wiring symbol"
    );
}