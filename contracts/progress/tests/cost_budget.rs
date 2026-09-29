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
//! These tests do not wire a registration contract (`set_registration_contract`
//! is intentionally left unset), so the measured cost reflects the progress
//! contract's own work only, not the cross-contract sync path — that path is
//! covered by the dedicated registration<->progress integration test instead.
//!
//! `advance_level` and `reset_player_level` both cover the Merkle commitment
//! cost added by issue #700 — recomputing the RFC 6962 Merkle Tree Hash over
//! the player's (already-materialized) history on every append. Budgets were
//! calibrated from real CI measurements with 20% headroom (see
//! `cpu-cost-budget-report.txt`).

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env};

const ADVANCE_LEVEL_CPU_BUDGET: u64 = 484_502;
const RESET_PLAYER_LEVEL_CPU_BUDGET: u64 = 639_231;
const GET_PROGRESS_HISTORY_PAGE_CPU_BUDGET: u64 = 195_802;
const LONG_HISTORY_ADVANCE_LEVEL_CPU_BUDGET: u64 = 30_000_000;
const VERIFY_HISTORY_PROOF_CPU_BUDGET: u64 = 139_669;

fn setup() -> (Env, ProgressContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let verification = Address::generate(&env);
    client.set_verification_contract(&verification);
    (env, client, verification)
}

/// Reads the CPU-instruction cost accumulated since the last budget reset
/// and asserts it is within `budget`, panicking with a diagnostic naming the
/// operation, the measured cost, and the overage when it is not.
fn assert_cpu_budget(env: &Env, op: &str, budget: u64) {
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();
    println!("cost_budget: progress::{op} = {cpu} cpu instructions (budget {budget})");
    assert!(
        cpu <= budget,
        "progress::{op} regressed: measured {cpu} cpu instructions, exceeding the \
         {budget}-instruction budget by {over} ({pct:.1}% over). See ci/cpu-cost-budget.md \
         for how to raise this budget if the growth is intentional.",
        over = cpu.saturating_sub(budget),
        pct = (cpu.saturating_sub(budget)) as f64 / budget as f64 * 100.0,
    );
}

#[test]
fn cost_advance_level() {
    let (env, client, verification) = setup();

    env.cost_estimate().budget().reset_default();
    client.advance_level(&verification, &1u64, &1u32);
    assert_cpu_budget(&env, "advance_level", ADVANCE_LEVEL_CPU_BUDGET);
}

#[test]
fn cost_reset_player_level() {
    let (env, client, verification) = setup();
    client.advance_level(&verification, &1u64, &1u32);

    env.cost_estimate().budget().reset_default();
    client.reset_player_level(&1u64, &ProgressLevel::Unverified);
    assert_cpu_budget(&env, "reset_player_level", RESET_PLAYER_LEVEL_CPU_BUDGET);
}

#[test]
fn cost_get_progress_history_page() {
    let (env, client, verification) = setup();
    client.advance_level(&verification, &1u64, &1u32);
    client.advance_level(&verification, &1u64, &2u32);

    env.cost_estimate().budget().reset_default();
    client.get_progress_history_page(&1u64, &0u32, &10u32);
    assert_cpu_budget(
        &env,
        "get_progress_history_page",
        GET_PROGRESS_HISTORY_PAGE_CPU_BUDGET,
    );
}

#[test]
fn cost_advance_level_stays_bounded_even_with_long_history() {
    let (env, client, verification) = setup();
    let player_id = 42u64;

    for i in 1..=24u32 {
        client.advance_level(&verification, &player_id, &i);
        if i % 3 == 0 {
            client.reset_player_level(&player_id, &ProgressLevel::Unverified);
        }
    }

    env.cost_estimate().budget().reset_default();
    client.advance_level(&verification, &player_id, &99u32);
    assert_cpu_budget(
        &env,
        "advance_level_long_history",
        LONG_HISTORY_ADVANCE_LEVEL_CPU_BUDGET,
    );
}

#[test]
fn cost_verify_history_proof() {
    let (env, client, verification) = setup();
    client.advance_level(&verification, &1u64, &1u32);
    client.advance_level(&verification, &1u64, &2u32);
    client.advance_level(&verification, &1u64, &3u32);

    let entry = client.get_history_entry(&1u64, &2u32);
    let proof = client.get_history_proof(&1u64, &2u32);

    env.cost_estimate().budget().reset_default();
    client.verify_history_proof(&1u64, &entry, &proof);
    assert_cpu_budget(
        &env,
        "verify_history_proof",
        VERIFY_HISTORY_PROOF_CPU_BUDGET,
    );
}
