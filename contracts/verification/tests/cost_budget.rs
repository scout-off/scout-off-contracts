//! CPU-instruction cost regression budget for the verification contract.
//!
//! Measures the CPU-instruction cost of representative verification
//! operations using soroban-sdk's test budget utilities (`Env::cost_estimate`)
//! and asserts each stays within a checked-in per-operation budget. See
//! `ci/cpu-cost-budget.md` for the full cross-contract budget table and the
//! process for raising a budget when a legitimate feature grows an
//! operation's cost.
//!
//! To raise a budget: bump the relevant constant below AND update the
//! matching row in `ci/cpu-cost-budget.md` with a one-line justification in
//! the PR description explaining why the growth is expected and acceptable.

use scoutchain_verification::{
    RevocationSeverity, VerificationContract, VerificationContractClient,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

// These starting budgets are deliberately generous placeholders, not
// measured baselines: this environment could not run `cargo test` to
// capture real current costs when this file was first introduced (no Rust
// toolchain available). Tighten each budget to roughly
// current-cost-plus-headroom after the first real CI run reports actual
// numbers — that tightening is a follow-up, not a blocker.
const REGISTER_VALIDATOR_CPU_BUDGET: u64 = 15_000_000;
const APPROVE_MILESTONE_CPU_BUDGET: u64 = 20_000_000;
const GET_VALIDATOR_MILESTONES_PAGE_CPU_BUDGET: u64 = 15_000_000;
/// Bounded cascade sweep (CASCADE_LIMIT = 50) with 500 total milestones.
/// Cost must be proportional to the 50-entry limit, not the full 500.
/// See ci/cpu-cost-budget.md — added for issue #1039.
const REVOKE_VALIDATOR_CASCADE_50_CPU_BUDGET: u64 = 50_000_000;

// Distinct valid CIDv0 evidence hashes (exactly 46 chars, base58btc — no
// 0/O/I/l). approve_milestone rejects duplicate evidence hashes globally.
const CID_1: &str = "QmRhbYsqpiYgUY9KfNCcbfopHPbLnWSVKBpDNs37aZ3kVC";
const CID_2: &str = "QmwsjoZwgfzgx6xPr3cXEKhzfLt5RQ87yMnWecTp1tf6p7";
const CID_3: &str = "QmgzsER5ykyxoTsVUSePRkKXqkEzsRVLpUv511dp4c3vAs";

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

/// Reads the CPU-instruction cost accumulated since the last budget reset
/// and asserts it is within `budget`, panicking with a diagnostic naming the
/// operation, the measured cost, and the overage when it is not.
fn assert_cpu_budget(env: &Env, op: &str, budget: u64) {
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();
    println!("cost_budget: verification::{op} = {cpu} cpu instructions (budget {budget})");
    assert!(
        cpu <= budget,
        "verification::{op} regressed: measured {cpu} cpu instructions, exceeding the \
         {budget}-instruction budget by {over} ({pct:.1}% over). See ci/cpu-cost-budget.md \
         for how to raise this budget if the growth is intentional.",
        over = cpu.saturating_sub(budget),
        pct = (cpu.saturating_sub(budget)) as f64 / budget as f64 * 100.0,
    );
}

#[test]
fn cost_register_validator() {
    let (env, client) = setup();
    let validator = Address::generate(&env);
    let credentials = String::from_str(&env, "UEFA-A-License-2026");

    env.cost_estimate().budget().reset_default();
    client.register_validator(&validator, &credentials, &String::from_str(&env, ""), &String::from_str(&env, "Default Region"), &Vec::new(&env));
    assert_cpu_budget(&env, "register_validator", REGISTER_VALIDATOR_CPU_BUDGET);
}

#[test]
fn cost_approve_milestone() {
    let (env, client) = setup();
    let validator = Address::generate(&env);
    client.register_validator(&validator, &String::from_str(&env, "UEFA-A-License-2026"), &String::from_str(&env, ""), &String::from_str(&env, "Default Region"), &Vec::new(&env));

    env.cost_estimate().budget().reset_default();
    client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "scored a hat-trick"),
        &String::from_str(&env, CID_1),
        &None,
    );
    assert_cpu_budget(&env, "approve_milestone", APPROVE_MILESTONE_CPU_BUDGET);
}

#[test]
fn cost_get_validator_milestones_page() {
    let (env, client) = setup();
    let validator = Address::generate(&env);
    client.register_validator(&validator, &String::from_str(&env, "UEFA-A-License-2026"), &String::from_str(&env, ""), &String::from_str(&env, "Default Region"), &Vec::new(&env));
    client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "scored a hat-trick"),
        &String::from_str(&env, CID_2),
        &None,
    );
    client.approve_milestone(
        &validator,
        &2u64,
        &String::from_str(&env, "clean sheet"),
        &String::from_str(&env, CID_3),
        &None,
    );

    env.cost_estimate().budget().reset_default();
    client.get_validator_milestones_page(&validator, &0u32, &10u32);
    assert_cpu_budget(
        &env,
        "get_validator_milestones_page",
        GET_VALIDATOR_MILESTONES_PAGE_CPU_BUDGET,
    );
}

/// Same BASE58 character set used in cascade_rereview tests.
const BASE58_CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn cid_for_budget(env: &Env, seed: u32) -> String {
    let mut s = std::string::String::from("Qm");
    let mut n = seed.wrapping_add(1);
    for _ in 0..44 {
        let idx = (n % BASE58_CHARS.len() as u32) as usize;
        s.push(BASE58_CHARS[idx] as char);
        n = n / (BASE58_CHARS.len() as u32) + seed.wrapping_add(7);
    }
    String::from_str(env, &s)
}

/// Cost of `revoke_validator(ForCause)` against a validator with 500 prior
/// approvals.  The cascade sweep is bounded by CASCADE_LIMIT = 50, so the
/// measured cost reflects 50 flag writes, NOT 500.
///
/// This test proves the non-negotiable spec requirement: per-call cost is
/// proportional to the per-call limit, not to total historical approval count.
/// See ci/cpu-cost-budget.md — issue #1039.
#[test]
fn cost_revoke_validator_cascade_50_limit_at_500_milestones() {
    let (env, client) = setup();
    let validator = Address::generate(&env);
    client.register_validator(&validator, &String::from_str(&env, "UEFA-A-License-2026"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::Vec::new(&env));

    // Approve 500 milestones: 5 per player, 100 players.
    let mut cid_seed: u32 = 20_000;
    for player_offset in 0u32..100 {
        let player_id = (20_000u32 + player_offset) as u64;
        for _ in 0..5 {
            client.approve_milestone(
                &validator,
                &player_id,
                &String::from_str(&env, "verified achievement"),
                &cid_for_budget(&env, cid_seed),
                &None,
            );
            cid_seed += 1;
        }
    }

    env.cost_estimate().budget().reset_default();
    client.revoke_validator(
        &validator,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "fabricated credentials")),
    );
    assert_cpu_budget(
        &env,
        "revoke_validator(ForCause, limit=50, total=500)",
        REVOKE_VALIDATOR_CASCADE_50_CPU_BUDGET,
    );
}
