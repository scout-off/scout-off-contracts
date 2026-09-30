//! CPU-instruction cost regression budget for the scout_access contract.
//!
//! Measures the CPU-instruction cost of representative scout_access
//! operations using soroban-sdk's test budget utilities (`Env::cost_estimate`)
//! and asserts each stays within a checked-in per-operation budget. See
//! `ci/cpu-cost-budget.md` for the full cross-contract budget table and the
//! process for raising a budget when a legitimate feature grows an
//! operation's cost.
//!
//! To raise a budget: bump the relevant constant below AND update the
//! matching row in `ci/cpu-cost-budget.md` with a one-line justification in
//! the PR description explaining why the growth is expected and acceptable.

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, String,
};

const SUBSCRIBE_CPU_BUDGET: u64 = 597_410;
// #619: pay_to_contact budget includes evidence_access_granted event emission
// (atomically written with every successful pay_to_contact call per
// docs/EVIDENCE_PRIVACY.md). Budget raised from 777,109 → 810,000 to cover
// the ~27k instruction increase from the event write.
const PAY_TO_CONTACT_CPU_BUDGET: u64 = 810_000;
// #619: batch_contact_players budget raised from 1,545,146 → 2,350,000 to
// cover the 5× evidence_access_granted event emissions (one per player)
// that are now written atomically alongside each contact record.
const BATCH_CONTACT_PLAYERS_CPU_BUDGET: u64 = 2_350_000;
// #795: expire_trial_offers is capped at 20 escrows/call — see
// EXPIRE_TRIAL_OFFERS_MAX_LIMIT in contracts/scout_access/src/lib.rs.
const EXPIRE_TRIAL_OFFERS_CPU_BUDGET: u64 = 8_614_029;
// #1040: get_player_access_grants CPU cost at 1000 total grants (paged index
// seek avoids a full scan). Measured at a mid-history page (offset 500,
// limit 50) — see cost_get_player_access_grants_at_1000_grants below.
const GET_PLAYER_ACCESS_GRANTS_CPU_BUDGET: u64 = 15_000_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
    }
}

fn setup() -> (Env, ScoutAccessContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm, &default_fees());
    (env, client, xlm)
}

fn fund(env: &Env, xlm: &Address, who: &Address) {
    StellarAssetClient::new(env, xlm).mint(who, &10_000_000i128);
}

/// Reads the CPU-instruction cost accumulated since the last budget reset
/// and asserts it is within `budget`, panicking with a diagnostic naming the
/// operation, the measured cost, and the overage when it is not.
fn assert_cpu_budget(env: &Env, op: &str, budget: u64) {
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();
    println!("cost_budget: scout_access::{op} = {cpu} cpu instructions (budget {budget})");
    assert!(
        cpu <= budget,
        "scout_access::{op} regressed: measured {cpu} cpu instructions, exceeding the \
         {budget}-instruction budget by {over} ({pct:.1}% over). See ci/cpu-cost-budget.md \
         for how to raise this budget if the growth is intentional.",
        over = cpu.saturating_sub(budget),
        pct = (cpu.saturating_sub(budget)) as f64 / budget as f64 * 100.0,
    );
}

#[test]
fn cost_subscribe() {
    let (env, client, xlm) = setup();
    let scout = Address::generate(&env);
    fund(&env, &xlm, &scout);

    env.cost_estimate().budget().reset_default();
    client.subscribe(&scout, &SubscriptionTier::Pro);
    assert_cpu_budget(&env, "subscribe", SUBSCRIBE_CPU_BUDGET);
}

#[test]
fn cost_pay_to_contact() {
    let (env, client, xlm) = setup();
    let scout = Address::generate(&env);
    fund(&env, &xlm, &scout);
    client.subscribe(&scout, &SubscriptionTier::Pro);

    env.cost_estimate().budget().reset_default();
    client.pay_to_contact(&scout, &1u64);
    assert_cpu_budget(&env, "pay_to_contact", PAY_TO_CONTACT_CPU_BUDGET);
}

#[test]
fn cost_batch_contact_players() {
    let (env, client, xlm) = setup();
    let scout = Address::generate(&env);
    fund(&env, &xlm, &scout);
    client.subscribe(&scout, &SubscriptionTier::Pro);
    let player_ids = vec![&env, 1u64, 2u64, 3u64, 4u64, 5u64];

    env.cost_estimate().budget().reset_default();
    client.batch_contact_players(&scout, &player_ids);
    assert_cpu_budget(
        &env,
        "batch_contact_players",
        BATCH_CONTACT_PLAYERS_CPU_BUDGET,
    );
}

#[test]
fn cost_expire_trial_offers() {
    let (env, client, xlm) = setup();
    let scout = Address::generate(&env);
    StellarAssetClient::new(&env, &xlm).mint(&scout, &50_000_000i128);
    client.subscribe(&scout, &SubscriptionTier::Elite);

    let hash = String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");
    for player_id in 1u64..=20u64 {
        client.pay_to_contact(&scout, &player_id);
        client.log_trial_offer(&scout, &player_id, &hash);
    }

    // Push all 20 escrows past their 1h expiry window.
    env.ledger().with_mut(|l| l.timestamp += 3_601);

    env.cost_estimate().budget().reset_default();
    let swept = client.expire_trial_offers(&20u32);
    assert_eq!(swept, 20);
    assert_cpu_budget(&env, "expire_trial_offers", EXPIRE_TRIAL_OFFERS_CPU_BUDGET);
}

/// #1040: proves `get_player_access_grants`' CPU cost is independent of a
/// player's total historical grant count. A single popular player
/// accumulates 1,000 grants from 1,000 distinct scouts, then a mid-history
/// page read (offset 500, spanning a page boundary) is measured — if the
/// query scanned the whole history instead of seeking directly to the
/// relevant page(s), this would blow well past the same budget a small
/// player's read costs.
#[test]
fn cost_get_player_access_grants_at_1000_grants() {
    let (env, client, xlm) = setup();
    let player_id = 1u64;

    for _ in 0..1_000u32 {
        let scout = Address::generate(&env);
        fund(&env, &xlm, &scout);
        client.subscribe(&scout, &SubscriptionTier::Elite);
        client.pay_to_contact(&scout, &player_id);
    }

    // Offset 500 with page size 50 lands mid-page (page 10, index 0) —
    // not a boundary-favorable case.
    env.cost_estimate().budget().reset_default();
    let page = client.get_player_access_grants(&player_id, &500u32, &50u32);
    assert_eq!(page.len(), 50);
    assert_cpu_budget(
        &env,
        "get_player_access_grants(1000 total, page of 50)",
        GET_PLAYER_ACCESS_GRANTS_CPU_BUDGET,
    );
}
