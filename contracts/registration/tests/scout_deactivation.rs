//! Tests for `deactivate_scout` / `reactivate_scout` and the resulting
//! status changes — issue #1403.
//!
//! Acceptance criteria:
//! 1. `deactivate_scout` sets the flag, emits `scout_deactivated` event.
//! 2. `reactivate_scout` clears the flag, emits `scout_reactivated` event.
//! 3. `get_scout_status` returns `Deactivated` / `Active` correctly.
//! 4. `is_scout_deactivated` boolean view reflects current state.
//! 5. Non-admin callers are rejected.
//! 6. Unknown scout_id returns ScoutNotFound.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient, ScoutStatus};
use soroban_sdk::testutils::{Address as _, Events, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, String, Val, Vec};

struct Harness {
    env: Env,
    admin: Address,
    client: RegistrationContractClient<'static>,
    contract_id: Address,
}

fn auth(env: &Env, address: &Address, contract_id: &Address, fn_name: &str, args: Vec<Val>) {
    env.mock_auths(&[MockAuth {
        address: address.clone(),
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

    // Register a scout to have something to deactivate
    let wallet = Address::generate(&env);
    auth(
        &env,
        &wallet,
        &contract_id,
        "register_scout",
        vec![&env, wallet.to_val(), String::from_str(&env, "EU").to_val()],
    );
    let scout_id = client.register_scout(&wallet, &String::from_str(&env, "EU"));

    Harness {
        env,
        admin,
        client,
        contract_id,
    }
}

/// Freshly registered scout starts as Active.
#[test]
fn test_fresh_scout_is_active() {
    let h = setup();
    // scout_id=1
    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Active);
    assert!(!h.client.is_scout_deactivated(&1));
}

/// Admin can deactivate a scout; status changes to Deactivated.
#[test]
fn test_deactivate_sets_deactivated_status() {
    let h = setup();
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);

    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Deactivated);
    assert!(h.client.is_scout_deactivated(&1));
}

/// Deactivating an already-deactivated scout is idempotent.
#[test]
fn test_deactivate_idempotent() {
    let h = setup();
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);

    // Second deactivate should also succeed
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);

    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Deactivated);
    assert!(h.client.is_scout_deactivated(&1));
}

/// Admin can reactivate a deactivated scout; status returns to Active.
#[test]
fn test_reactivate_clears_deactivated_status() {
    let h = setup();

    // Deactivate first
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);
    assert!(h.client.is_scout_deactivated(&1));

    // Now reactivate
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "reactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.reactivate_scout(&1);

    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Active);
    assert!(!h.client.is_scout_deactivated(&1));
}

/// Reactivating an already-active scout is idempotent.
#[test]
fn test_reactivate_idempotent() {
    let h = setup();

    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "reactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.reactivate_scout(&1);

    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Active);
    assert!(!h.client.is_scout_deactivated(&1));
}

/// Non-admin caller is rejected.
#[test]
fn test_deactivate_rejects_non_admin() {
    let h = setup();
    let random = Address::generate(&h.env);

    auth(
        &h.env,
        &random,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    let result = h.client.try_deactivate_scout(&1);
    assert!(result.is_err(), "non-admin must be rejected");
}

/// Unknown scout_id returns ScoutNotFound.
#[test]
fn test_deactivate_unknown_scout() {
    let h = setup();

    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (999u64).to_val()],
    );
    let result = h.client.try_deactivate_scout(&999);
    assert!(result.is_err(), "unknown scout must be rejected");
}

/// `deactivate_scout` emits the `scout_deactivated` event.
#[test]
fn test_deactivate_emits_event() {
    let h = setup();

    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);

    let events = h.env.events().all();
    let found = events.iter().any(|e| {
        e.0 == h.contract_id
            && e.1.to_string().contains("scout_deactivated")
    });
    assert!(found, "scout_deactivated event must be emitted");
}

/// `reactivate_scout` emits the `scout_reactivated` event.
#[test]
fn test_reactivate_emits_event() {
    let h = setup();

    // Deactivate first
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);

    // Reactivate
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "reactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.reactivate_scout(&1);

    let events = h.env.events().all();
    let found = events.iter().any(|e| {
        e.0 == h.contract_id
            && e.1.to_string().contains("scout_reactivated")
    });
    assert!(found, "scout_reactivated event must be emitted");
}

/// `is_scout_deactivated` returns false for non-existent scout.
#[test]
fn test_is_deactivated_false_for_missing() {
    let h = setup();
    assert!(!h.client.is_scout_deactivated(&999));
}

/// Full round-trip: Active -> Deactivated -> Active.
#[test]
fn test_full_round_trip() {
    let h = setup();

    // Start: Active
    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Active);

    // Deactivate
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);
    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Deactivated);

    // Reactivate
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "reactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.reactivate_scout(&1);
    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Active);

    // Deactivate again
    auth(
        &h.env,
        &h.admin,
        &h.contract_id,
        "deactivate_scout",
        vec![&h.env, (1u64).to_val()],
    );
    h.client.deactivate_scout(&1);
    assert_eq!(h.client.get_scout_status(&1), ScoutStatus::Deactivated);
}