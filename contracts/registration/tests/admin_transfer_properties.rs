//! Property-based tests for the admin propose/accept flow (#824).
//!
//! These tests verify the same invariant set:
//! 1. `accept_admin` fails unless the pending (proposed) address authorizes it.
//! 2. Double `propose_admin` replaces, never queues, the pending proposal.
//! 3. Admin is immutable except via successful `accept_admin`.
//! 4. A replaced proposal cannot be accepted by the old proposed address.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, Val, Vec};

struct Harness {
    env: Env,
    admin: Address,
    client: RegistrationContractClient<'static>,
    contract_id: Address,
}

/// Mock a single authorization entry for the given address + invocation.
fn auth(env: &Env, address: &Address, contract_id: &Address, fn_name: &str, args: Vec<Val>) {
    env.mock_auths(&[MockAuth {
        address,
        invoke: &MockAuthInvoke {
            contract: contract_id,
            fn_name,
            args,
            sub_invokes: &[],
        },
    }]);
}

fn setup() -> Harness {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &contract_id);

    auth(
        &env,
        &admin,
        &contract_id,
        "initialize",
        vec![&env, admin.to_val()],
    );
    client.initialize(&admin);

    Harness {
        env,
        admin,
        client,
        contract_id,
    }
}

fn propose(h: &Harness, new_admin: &Address) {
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "propose_admin",
        vec![&h.env, new_admin.to_val()],
    );
    h.client.propose_admin(new_admin);
}

// Property 1: only the proposed address can accept

#[test]
fn test_only_proposed_can_accept() {
    let h = setup();
    let proposed = Address::generate(&h.env);
    propose(&h, &proposed);

    auth(
        &h.env,
        &proposed,
        &h.contract_id,
        "accept_admin",
        vec![&h.env],
    );
    let result = h.client.try_accept_admin();
    assert!(result.is_ok());
}

#[test]
fn test_non_proposed_cannot_accept() {
    let h = setup();
    let proposed = Address::generate(&h.env);
    propose(&h, &proposed);

    // No auth entry for accept_admin → require_auth on the pending address
    // traps, so the call must fail.
    let result = h.client.try_accept_admin();
    assert!(result.is_err());
}

// Property 2: double propose replaces

#[test]
fn test_double_propose_replaces_pending() {
    let h = setup();
    let first = Address::generate(&h.env);
    let second = Address::generate(&h.env);
    propose(&h, &first);
    propose(&h, &second);

    // The first proposal was replaced; first can no longer accept.
    auth(&h.env, &first, &h.contract_id, "accept_admin", vec![&h.env]);
    let result = h.client.try_accept_admin();
    assert!(result.is_err());

    auth(
        &h.env,
        &second,
        &h.contract_id,
        "accept_admin",
        vec![&h.env],
    );
    let result2 = h.client.try_accept_admin();
    assert!(result2.is_ok());
}

// Property 3: admin unchanged until accept

#[test]
fn test_admin_unchanged_before_accept() {
    let h = setup();
    let new_admin = Address::generate(&h.env);
    propose(&h, &new_admin);

    // The old admin is still in control: a second proposal (admin-only)
    // succeeds, proving admin did not change.
    let third = Address::generate(&h.env);
    propose(&h, &third);

    auth(&h.env, &third, &h.contract_id, "accept_admin", vec![&h.env]);
    let result = h.client.try_accept_admin();
    assert!(result.is_ok());
}

// Property 4: replaced proposal cannot be accepted

#[test]
fn test_replaced_proposal_cannot_accept() {
    let h = setup();
    let first = Address::generate(&h.env);
    let second = Address::generate(&h.env);
    propose(&h, &first);
    propose(&h, &second);

    auth(&h.env, &first, &h.contract_id, "accept_admin", vec![&h.env]);
    let result = h.client.try_accept_admin();
    assert!(result.is_err());
}
