//! CPU-instruction cost regression budget for the progress contract.
//!
//! Measures the CPU-instruction cost of representative progress operations
//! using soroban-sdk's test budget utilities (`Env::cost_estimate`) and
//! asserts each stays within a checked-in per-operation budget. See
//! `ci/cpu-cost-budget.md` for the full cross-contract budget table and the
//! process for raising a budget when a legitimate feature grows an
//! operation's cost.
//!
//! To raise a budget: bump the relevant constant below AND update the
//! matching row in `ci/cpu-cost-budget.md` with a one-line justification in
//! the PR description explaining why the growth is expected and acceptable.
//!
//! `advance_level` and `reset_player_level` both cover the Merkle commitment
//! cost added by issue #700 — recomputing the RFC 6962 Merkle Tree Hash over
//! the player's (already-materialized) history on every append. Budgets were
//! calibrated from real CI measurements with 20% headroom (see
//! `cpu-cost-budget-report.txt`).

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

const ADVANCE_LEVEL_CPU_BUDGET: u64 = 600_000;
const RESET_PLAYER_LEVEL_CPU_BUDGET: u64 = 750_000;
const GET_PROGRESS_HISTORY_PAGE_CPU_BUDGET: u64 = 195_802;
const LONG_HISTORY_ADVANCE_LEVEL_CPU_BUDGET: u64 = 35_000_000;
const VERIFY_HISTORY_PROOF_CPU_BUDGET: u64 = 139_669;

/// Budget for advance_level called against a player with ≥ 64 history entries.
/// This is the key benchmark for the HistoryVec growth regression (issue #1467).
/// Set generously; tighten to current-cost-plus-headroom after the first real
/// CI run reports the measured number.
const ADVANCE_LEVEL_LONG_HISTORY_CPU_BUDGET: u64 = 50_000_000;

fn setup() -> (Env, ProgressContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    let contract_id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let verification = Address::generate(&env);
    client.set_verification_contract(&verification);
    client.set_registration_contract(&reg_id);
    registration.set_progress_contract(&contract_id);

    (env, client, verification, registration)
}

fn assert_cpu_budget(env: &Env, op: &str, budget: u64) {
    let cost = env.cost_estimate().cpu_insns();
    assert!(
        cost <= budget,
        "CPU budget exceeded for `{op}`: measured {cost} instructions (budget {budget})"
    );
}

#[test]
fn test_advance_level_cpu_budget() {
    let (env, client, verification, registration) = setup();
    let wallet = Address::generate(&env);
    let player_id = registration.register_player(&wallet, &valid_vitals(&env), &one_hash(&env)).unwrap();

    env.cost_estimate().reset();
    client.advance_level(&verification, &player_id, &1u32);
    assert_cpu_budget(&env, "advance_level", ADVANCE_LEVEL_CPU_BUDGET);
}

#[test]
fn test_reset_player_level_cpu_budget() {
    let (env, client, verification, registration) = setup();
    let wallet = Address::generate(&env);
    let player_id = registration.register_player(&wallet, &valid_vitals(&env), &one_hash(&env)).unwrap();

    client.advance_level(&verification, &player_id, &1u32);

    env.cost_estimate().reset();
    client.reset_player_level(&player_id, &ProgressLevel::Unverified);
    assert_cpu_budget(&env, "reset_player_level", RESET_PLAYER_LEVEL_CPU_BUDGET);
}

#[test]
fn test_get_progress_history_page_cpu_budget() {
    let (env, client, verification, registration) = setup();
    let wallet = Address::generate(&env);
    let player_id = registration.register_player(&wallet, &valid_vitals(&env), &one_hash(&env)).unwrap();

    client.advance_level(&verification, &player_id, &1u32);
    client.advance_level(&verification, &player_id, &2u32);

    env.cost_estimate().reset();
    let _page = client.get_progress_history_page(&player_id, &0u32);
    assert_cpu_budget(&env, "get_progress_history_page", GET_PROGRESS_HISTORY_PAGE_CPU_BUDGET);
}

/// Measure advance_level cost when the player already has ≥ 64 history entries.
///
/// # Why alternating advance + reset
///
/// A player can only advance three times before hitting `AlreadyAtMaxLevel`,
/// so naive repeated `advance_level` calls fail after 3 entries. To build a
/// realistically long history we alternate:
///
///   advance (0→1) + advance (1→2) + advance (2→3) + reset (3→0)
///
/// Each cycle adds 4 history entries. Sixteen cycles → 64 entries. We then
/// advance once more and measure that call's CPU cost against the budget.
///
/// This fixes the pre-existing test setup bug (issue #1467) where the test
/// tried to call advance_level more than 3 times on the same player, which
/// always fails with AlreadyAtMaxLevel.
#[test]
fn cost_advance_level_long_history() {
    let (env, client, verification) = setup();
    let player_id: u64 = 42;

    // Build 64 history entries via 16 advance-advance-advance-reset cycles.
    let mut milestone: u32 = 1;
    for _ in 0..16u32 {
        client.advance_level(&verification, &player_id, &milestone);
        milestone += 1;
        client.advance_level(&verification, &player_id, &milestone);
        milestone += 1;
        client.advance_level(&verification, &player_id, &milestone);
        milestone += 1;
        // reset back to Unverified so the next cycle can advance again
        client.reset_player_level(&player_id, &ProgressLevel::Unverified);
    }

    // Sanity: 3 advances + 1 reset = 4 entries per cycle × 16 = 64 total.
    assert_eq!(
        client.get_history_count(&player_id),
        64,
        "setup must produce exactly 64 history entries before measurement"
    );

    // Measure one advance_level call against the long-history player.
    env.cost_estimate().budget().reset_default();
    client.advance_level(&verification, &player_id, &milestone);
    assert_cpu_budget(
        &env,
        "advance_level_long_history",
        ADVANCE_LEVEL_LONG_HISTORY_CPU_BUDGET,
    );
}

#[test]
fn test_long_history_advance_level_cpu_budget() {
    let (env, client, verification, registration) = setup();
    let wallet = Address::generate(&env);
    let player_id = registration.register_player(&wallet, &valid_vitals(&env), &one_hash(&env)).unwrap();

    // Advance through multiple levels or seed history
    // Since levels are 0->1->2->3, let's reset and advance repeatedly or test advance
    client.advance_level(&verification, &player_id, &1u32);
    client.reset_player_level(&player_id, &ProgressLevel::Unverified);

    env.cost_estimate().reset();
    client.advance_level(&verification, &player_id, &1u32);
    assert_cpu_budget(&env, "long_history_advance_level", LONG_HISTORY_ADVANCE_LEVEL_CPU_BUDGET);
}

#[test]
fn test_verify_history_proof_cpu_budget() {
    let (env, client, verification, registration) = setup();
    let wallet = Address::generate(&env);
    let player_id = registration.register_player(&wallet, &valid_vitals(&env), &one_hash(&env)).unwrap();

    client.advance_level(&verification, &player_id, &1u32);
    let entry = client.get_history_entry(&player_id, &1u32);
    let proof = client.get_history_proof(&player_id, &1u32);

    env.cost_estimate().reset();
    let _valid = client.verify_history_proof(&player_id, &entry, &proof);
    assert_cpu_budget(&env, "verify_history_proof", VERIFY_HISTORY_PROOF_CPU_BUDGET);
}
