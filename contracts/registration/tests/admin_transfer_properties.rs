//! Property-based tests for the admin propose/accept flow (#824).
//!
//! These tests verify the same invariant set:
//! 1. `accept_admin` fails unless the pending (proposed) address authorizes it.
//! 2. Double `propose_admin` replaces, never queues, the pending proposal.
//! 3. Admin is immutable except via successful `accept_admin`.
//! 4. A replaced proposal cannot be accepted by the old proposed address.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, Symbol, Val, Vec};

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
    let contract_id = env.register(RegistrationContract, (admin.clone(),));
    let client = RegistrationContractClient::new(&env, &contract_id);

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

fn transfer(h: &Harness, new_admin: &Address) {
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "transfer_admin",
        vec![&h.env, new_admin.to_val()],
    );
    h.client.transfer_admin(new_admin);
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

// transfer_admin alias: only proposes, never transfers immediately (#1887)

#[test]
fn test_transfer_admin_only_proposes() {
    let h = setup();
    let new_admin = Address::generate(&h.env);
    transfer(&h, &new_admin);

    // Admin is still the original admin: it can still call an admin-only fn.
    let third = Address::generate(&h.env);
    propose(&h, &third);

    // The pending admin is the one proposed via transfer_admin.
    auth(
        &h.env,
        &new_admin,
        &h.contract_id,
        "accept_admin",
        vec![&h.env],
    );
    let result = h.client.try_accept_admin();
    assert!(result.is_ok());
}

#[test]
fn test_transfer_admin_requires_accept() {
    let h = setup();
    let new_admin = Address::generate(&h.env);
    transfer(&h, &new_admin);

    // Without the pending admin's auth, accept_admin must fail.
    let result = h.client.try_accept_admin();
    assert!(result.is_err());

    // Admin is still the original admin: it can still propose.
    let third = Address::generate(&h.env);
    propose(&h, &third);

    // With the pending admin's auth, accept_admin succeeds.
    auth(
        &h.env,
        &new_admin,
        &h.contract_id,
        "accept_admin",
        vec![&h.env],
    );
    let result2 = h.client.try_accept_admin();
    assert!(result2.is_ok());
}

#[test]
fn test_transfer_admin_non_admin_rejected() {
    let h = setup();
    let non_admin = Address::generate(&h.env);
    let new_admin = Address::generate(&h.env);

    auth(
        &h.env,
        &non_admin,
        &h.contract_id,
        "transfer_admin",
        vec![&h.env, new_admin.to_val()],
    );
    let result = h.client.try_transfer_admin(&new_admin);
    assert!(result.is_err());
}

#[test]
fn test_transfer_admin_emits_propose_event() {
    let h = setup();
    let new_admin = Address::generate(&h.env);

    transfer(&h, &new_admin);
    let transfer_events = h.env.events().all();

    let h2 = setup();
    let new_admin2 = Address::generate(&h2.env);
    propose(&h2, &new_admin2);
    let propose_events = h2.env.events().all();

    assert_eq!(transfer_events.len(), propose_events.len());
    for i in 0..transfer_events.len() {
        let (t_contract, t_topics, t_data) = transfer_events.get(i).unwrap();
        let (p_contract, p_topics, p_data) = propose_events.get(i).unwrap();
        assert_eq!(t_contract, p_contract);
        assert_eq!(t_topics, p_topics);
        assert_eq!(t_data, p_data);
    }
}

// contract_kind is pinned for the wiring handshake (#1888)

#[test]
fn test_contract_kind() {
    let h = setup();
    assert_eq!(
        h.client.contract_kind(),
        Symbol::new(&h.env, "registration")
    );
}
