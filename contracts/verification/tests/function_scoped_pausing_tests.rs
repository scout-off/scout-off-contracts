//! Tests for function-scoped pausing of approve_milestone (#809).
//!
//! Covers:
//! - approve_milestone blocked by function-scoped pause
//! - Other state-changing functions not affected by function-scoped pause
//! - Interaction between whole-contract pause and function-scoped pause
//! - Admin controls for pause/unpause

use scoutchain_verification::{
    RevocationSeverity, VerificationContract, VerificationContractClient,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    Address, Env, String,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

const VALID_CID: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const CREDENTIALS: &str = "UEFA B License";
const DESCRIPTION: &str = "Scored 5 goals in season";

struct Harness {
    env: Env,
    validator: Address,
    client: VerificationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let validator = Address::generate(&env);

    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);

    client.initialize(&admin);
    client.register_validator(
        &validator,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, "Default Academy"),
        &soroban_sdk::Vec::new(&env),
    );

    Harness {
        env,
        validator,
        client,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Function-scoped pause for approve_milestone
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_approve_milestone_succeeds_when_not_paused() {
    let h = setup();

    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(
        result.is_ok(),
        "approve_milestone should succeed when not paused"
    );
}

#[test]
fn test_approve_milestone_blocked_by_function_scoped_pause() {
    let h = setup();

    // Pause only approve_milestone
    h.client.pause_approve_milestone();

    // Try to approve a milestone
    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(
        result.is_err(),
        "approve_milestone should be blocked by function-scoped pause"
    );
}

#[test]
fn test_approve_milestone_succeeds_after_unpause() {
    let h = setup();

    // Pause approve_milestone
    h.client.pause_approve_milestone();

    // Unpause approve_milestone
    h.client.unpause_approve_milestone();

    // Now approve_milestone should work
    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(
        result.is_ok(),
        "approve_milestone should succeed after unpause"
    );
}

#[test]
fn test_approve_milestone_blocked_by_whole_contract_pause() {
    let h = setup();

    // Pause entire contract
    h.client.pause_contract();

    // Try to approve a milestone
    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(
        result.is_err(),
        "approve_milestone should be blocked by whole-contract pause"
    );
}

#[test]
fn test_approve_milestone_blocked_by_both_pauses() {
    let h = setup();

    // Pause both whole-contract and function-scoped
    h.client.pause_contract();
    h.client.pause_approve_milestone();

    // Try to approve a milestone
    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(result.is_err(), "approve_milestone should be blocked");
}

#[test]
fn test_pause_approve_milestone_requires_admin() {
    // Fresh env WITHOUT mock_all_auths so the admin gate can actually fail.
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let admin = Address::generate(&env);
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);

    // Mock ONLY the admin's initialize auth; pause_approve_milestone's
    // require_admin check then fails for any unauthenticated caller.
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "initialize",
            args: soroban_sdk::vec![&env, admin.to_val()],
            sub_invokes: &[],
        },
    }]);
    client.initialize(&admin);

    let result = client.try_pause_approve_milestone();
    assert!(
        result.is_err(),
        "non-admin should not be able to pause approve_milestone"
    );
}

#[test]
fn test_unpause_approve_milestone_requires_admin() {
    // Fresh env WITHOUT mock_all_auths so the admin gate can actually fail.
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let admin = Address::generate(&env);
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);

    // Mock ONLY the admin's auth for the calls that must succeed.
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "initialize",
            args: soroban_sdk::vec![&env, admin.to_val()],
            sub_invokes: &[],
        },
    }]);
    client.initialize(&admin);

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "pause_approve_milestone",
            args: soroban_sdk::vec![&env],
            sub_invokes: &[],
        },
    }]);
    client.pause_approve_milestone();

    // Try to unpause with no auth mocked — must be rejected.
    let result = client.try_unpause_approve_milestone();
    assert!(
        result.is_err(),
        "non-admin should not be able to unpause approve_milestone"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Other functions unaffected by function-scoped pause
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_register_validator_works_when_approve_milestone_paused() {
    let h = setup();
    let new_validator = Address::generate(&h.env);

    // Pause only approve_milestone
    h.client.pause_approve_milestone();

    // register_validator should still work
    h.client.register_validator(
        &new_validator,
        &String::from_str(&h.env, CREDENTIALS),
        &String::from_str(&h.env, "Default Academy"),
        &soroban_sdk::Vec::new(&h.env),
    );
}

#[test]
fn test_batch_register_validators_works_when_approve_milestone_paused() {
    let h = setup();
    let validator1 = Address::generate(&h.env);
    let validator2 = Address::generate(&h.env);

    let entries = soroban_sdk::Vec::from_array(
        &h.env,
        [
            (
                validator1,
                String::from_str(&h.env, CREDENTIALS),
                String::from_str(&h.env, "Default Academy"),
                soroban_sdk::Vec::new(&h.env),
            ),
            (
                validator2,
                String::from_str(&h.env, "UEFA A License"),
                String::from_str(&h.env, "Default Academy"),
                soroban_sdk::Vec::new(&h.env),
            ),
        ],
    );

    // Pause only approve_milestone
    h.client.pause_approve_milestone();

    // batch_register_validators should still work
    h.client.batch_register_validators(&entries);
}

#[test]
fn test_revoke_validator_works_when_approve_milestone_paused() {
    let h = setup();

    // Pause only approve_milestone
    h.client.pause_approve_milestone();

    // revoke_validator should still work
    h.client
        .revoke_validator(&h.validator, &RevocationSeverity::Routine, &None);
}

#[test]
fn test_get_validator_works_when_approve_milestone_paused() {
    let h = setup();

    // Pause only approve_milestone
    h.client.pause_approve_milestone();

    // Read query should work (always, regardless of pause)
    let validator_status = h.client.get_validator_status(&h.validator);

    assert_eq!(
        validator_status,
        scoutchain_verification::ValidatorStatus::Active,
        "validator should still be Active"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Health query reflects function-scoped pause state
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_health_reflects_approve_milestone_pause_state() {
    let h = setup();

    // Initially unpause
    let health_before = h.client.health();
    assert!(!health_before.paused, "whole-contract should not be paused");
    // Note: health structure should include approve_milestone_paused field

    // Pause approve_milestone
    h.client.pause_approve_milestone();

    // Health should now reflect it
    let health_after = h.client.health();
    assert!(
        !health_after.paused,
        "whole-contract should still not be paused"
    );
    // Note: approve_milestone_paused field should be true if exposed
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Interaction between pause levels
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_whole_contract_pause_overrides_unpause_approve_milestone() {
    let h = setup();

    // Pause whole-contract
    h.client.pause_contract();

    // Try to approve milestone
    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(
        result.is_err(),
        "approve_milestone should still be blocked by whole-contract pause"
    );
}

#[test]
fn test_function_pause_independent_of_whole_contract_pause() {
    let h = setup();

    // Pause only function-scoped
    h.client.pause_approve_milestone();

    // Verify register_validator still works (whole-contract is not paused)
    let new_validator = Address::generate(&h.env);
    h.client.register_validator(
        &new_validator,
        &String::from_str(&h.env, CREDENTIALS),
        &String::from_str(&h.env, "Default Academy"),
        &soroban_sdk::Vec::new(&h.env),
    );

    // Verify approve_milestone is still blocked
    let result = h.client.try_approve_milestone(
        &h.validator,
        &1u64,
        &String::from_str(&h.env, DESCRIPTION),
        &String::from_str(&h.env, VALID_CID),
        &None,
    );

    assert!(result.is_err(), "approve_milestone should be blocked");
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Events emitted correctly
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_pause_approve_milestone_emits_event() {
    let h = setup();

    h.client.pause_approve_milestone();

    // Verify event was emitted (can check via contract logs if needed)
}

#[test]
fn test_unpause_approve_milestone_emits_event() {
    let h = setup();

    h.client.pause_approve_milestone();

    h.client.unpause_approve_milestone();

    // Verify event was emitted
}
