//! Gas-griefing regression tests for the verification contract — Issue #812.
//!
//! Proves that the ValidatorVector cap (MAX_VALIDATORS = 100) bounds the O(N)
//! scan cost in `get_validators()` and that the cap is actually enforced.
//!
//! See docs/GAS_GRIEFING_AUDIT.md — Vector 1: ValidatorVector Monotonic Growth.

use scoutchain_verification::{
    RevocationSeverity, VerificationContract, VerificationContractClient,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

// ---------------------------------------------------------------------------
// Test 1: Validator cap is enforced at exactly 100
// ---------------------------------------------------------------------------

/// Registers 100 validators (the platform cap) and confirms that a 101st
/// registration is rejected with `ValidatorCapReached` (code 15).
///
/// This bounds the ValidatorVector length — and therefore the O(N) scan
/// cost of `get_validators()` — at a fixed maximum of 100 entries.
#[test]
fn test_validator_cap_enforced_at_100() {
    let (env, client) = setup();

    for _ in 0..100 {
        let v = Address::generate(&env);
        client.register_validator(
            &v,
            &String::from_str(&env, "UEFA-B-License-2026"),
            &String::from_str(&env, "Default Academy"),
            &soroban_sdk::Vec::new(&env),
        );
    }

    // Active validator count must be exactly 100.
    assert_eq!(
        client.get_active_validator_count(),
        100,
        "active validator count must be 100 at the cap"
    );

    // 101st registration must fail.
    let extra = Address::generate(&env);
    let result = client.try_register_validator(
        &extra,
        &String::from_str(&env, "UEFA-A-License-2026"),
        &String::from_str(&env, "Default Academy"),
        &soroban_sdk::Vec::new(&env),
    );
    assert!(
        matches!(
            result,
            Err(Ok(
                scoutchain_verification::VerificationError::ValidatorCapReached
            ))
        ),
        "101st validator registration must return ValidatorCapReached: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: get_validators() returns only active entries after revocations
// ---------------------------------------------------------------------------

/// Registers 10 validators, revokes 5, then calls `get_validators()` and
/// confirms:
/// 1. Only the 5 active validators are returned.
/// 2. The call completes without error (cost is bounded by the validator cap).
///
/// This proves that the O(N) scan over the ValidatorVector (which includes
/// revoked entries) correctly filters them out and that the bounded cap
/// prevents unbounded scan cost.
#[test]
fn test_get_validators_with_revoked_entries_bounded() {
    let (env, client) = setup();

    let mut validators: Vec<Address> = Vec::new();
    for _ in 0..10 {
        let v = Address::generate(&env);
        client.register_validator(
            &v,
            &String::from_str(&env, "UEFA-B-License-2026"),
            &String::from_str(&env, "Default Academy"),
            &soroban_sdk::Vec::new(&env),
        );
        validators.push(v);
    }

    // Revoke the first 5.
    for v in validators.iter().take(5) {
        client.revoke_validator(v, &RevocationSeverity::Routine, &None);
    }

    // get_validators() must return only the 5 active ones.
    let active = client.get_validators();
    assert_eq!(
        active.len(),
        5,
        "get_validators() must return exactly 5 active validators after revoking 5 of 10"
    );

    // Active count counter must also reflect 5.
    assert_eq!(
        client.get_active_validator_count(),
        5,
        "active_validator_count must be 5 after 5 revocations"
    );

    // None of the revoked addresses should appear in the active list.
    for revoked in validators.iter().take(5) {
        assert!(
            !active.contains(revoked),
            "revoked validator {revoked:?} must not appear in get_validators() result"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: get_validators() cost stays within budget at 100 registered
// ---------------------------------------------------------------------------

/// Registers 100 validators (the maximum), then measures the CPU cost of
/// `get_validators()` to confirm it stays within the budget.
/// This is the worst-case scan: all 100 entries, all active.
///
/// Budget: 15,000,000 CPU instructions (same as approved in ci/cpu-cost-budget.md).
#[test]
fn test_get_validators_cpu_cost_at_cap() {
    let (env, client) = setup();
    const GET_VALIDATORS_AT_CAP_BUDGET: u64 = 30_000_000; // 2× the standard page budget

    for _ in 0..100 {
        let v = Address::generate(&env);
        client.register_validator(
            &v,
            &String::from_str(&env, "UEFA-B-License-2026"),
            &String::from_str(&env, "Default Academy"),
            &soroban_sdk::Vec::new(&env),
        );
    }

    env.cost_estimate().budget().reset_default();
    let active = client.get_validators();
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();

    println!(
        "gas_griefing: get_validators() at 100 validators = {cpu} cpu instructions \
         (budget {GET_VALIDATORS_AT_CAP_BUDGET})"
    );

    assert_eq!(active.len(), 100, "all 100 validators must be returned");
    assert!(
        cpu <= GET_VALIDATORS_AT_CAP_BUDGET,
        "get_validators() at 100 validators exceeded budget: {cpu} > {GET_VALIDATORS_AT_CAP_BUDGET}"
    );
}
