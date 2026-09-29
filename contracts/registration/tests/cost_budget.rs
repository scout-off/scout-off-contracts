//! CPU-instruction cost regression budget for the registration contract.
//!
//! Measures the CPU-instruction cost of representative registration
//! operations using soroban-sdk's test budget utilities (`Env::cost_estimate`)
//! and asserts each stays within a checked-in per-operation budget. See
//! `ci/cpu-cost-budget.md` for the full cross-contract budget table and the
//! process for raising a budget when a legitimate feature grows an
//! operation's cost.
//!
//! To raise a budget: bump the relevant constant below AND update the
//! matching row in `ci/cpu-cost-budget.md` with a one-line justification in
//! the PR description explaining why the growth is expected and acceptable.

use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

const REGISTER_PLAYER_CPU_BUDGET: u64 = 452_787;
const UPDATE_PROFILE_CPU_BUDGET: u64 = 193_686;
const FILTER_PLAYERS_CPU_BUDGET: u64 = 1_714_870;

fn setup() -> (Env, RegistrationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn dummy_vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Midfielder"),
        region: String::from_str(env, "West Africa"),
        nationality: String::from_str(env, "Nigeria"),
    }
}

/// Reads the CPU-instruction cost accumulated since the last budget reset
/// and asserts it is within `budget`, panicking with a diagnostic naming the
/// operation, the measured cost, and the overage when it is not.
fn assert_cpu_budget(env: &Env, op: &str, budget: u64) {
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();
    println!("cost_budget: registration::{op} = {cpu} cpu instructions (budget {budget})");
    assert!(
        cpu <= budget,
        "registration::{op} regressed: measured {cpu} cpu instructions, exceeding the \
         {budget}-instruction budget by {over} ({pct:.1}% over). See ci/cpu-cost-budget.md \
         for how to raise this budget if the growth is intentional.",
        over = cpu.saturating_sub(budget),
        pct = (cpu.saturating_sub(budget)) as f64 / budget as f64 * 100.0,
    );
}

#[test]
fn cost_register_player() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let vitals = dummy_vitals(&env);
    let hashes = vec![&env, String::from_str(&env, "QmTestHashRegisterPlayer1")];

    env.cost_estimate().budget().reset_default();
    client.register_player(&wallet, &vitals, &hashes);
    assert_cpu_budget(&env, "register_player", REGISTER_PLAYER_CPU_BUDGET);
}

#[test]
fn cost_update_profile() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let vitals = dummy_vitals(&env);
    let hashes = vec![&env, String::from_str(&env, "QmTestHashUpdateProfile1")];
    let player_id = client.register_player(&wallet, &vitals, &hashes);
    let new_hashes = vec![&env, String::from_str(&env, "QmTestHashUpdateProfile2")];

    env.cost_estimate().budget().reset_default();
    client.update_profile(&player_id, &new_hashes);
    assert_cpu_budget(&env, "update_profile", UPDATE_PROFILE_CPU_BUDGET);
}

#[test]
fn cost_filter_players() {
    let (env, client) = setup();
    let region = String::from_str(&env, "West Africa");
    for i in 0..10u32 {
        let wallet = Address::generate(&env);
        let vitals = dummy_vitals(&env);
        let hashes = vec![
            &env,
            String::from_str(&env, &format!("QmTestHashFilterPlayers{i}")),
        ];
        client.register_player(&wallet, &vitals, &hashes);
    }

    env.cost_estimate().budget().reset_default();
    client.filter_players(
        &region,
        &String::from_str(&env, ""),
        &ProgressLevel::Unverified,
        &0u32,
        &20u32,
    );
    assert_cpu_budget(&env, "filter_players", FILTER_PLAYERS_CPU_BUDGET);
}
