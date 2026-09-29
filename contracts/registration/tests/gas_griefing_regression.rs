//! Gas-griefing regression tests for the registration contract — Issue #812.
//!
//! Proves that `filter_players` per-call cost is bounded by the internal
//! limit cap (50 results per call) regardless of how many players are
//! registered in a given bucket.
//!
//! See docs/GAS_GRIEFING_AUDIT.md — Vector 2: register_player Spam Inflates
//! filter_players Cost.

pub use scoutchain_registration::PlayerVitals;
use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

fn setup() -> (Env, RegistrationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    // These tests register 50–60 players and paginate through them; the test
    // env's default mainnet-style invocation resource limits (100 footprint
    // ledger entries) would reject that much state, so disable them. The CPU
    // cost budget is asserted separately per test below.
    env.host().set_invocation_resource_limits(None).unwrap();
    let contract_id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn dummy_vitals(env: &Env, position: &str, region: &str) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, position),
        region: String::from_str(env, region),
        nationality: String::from_str(env, "Test"),
    }
}

fn dummy_hashes(env: &Env) -> Vec<String> {
    let mut v = Vec::new(env);
    v.push_back(String::from_str(
        env,
        "QmRhbYsqpiYgUY9KfNCcbfopHPbLnWSVKBpDNs37aZ3kVC",
    ));
    v
}

// ---------------------------------------------------------------------------
// Test 1: filter_players page limit is enforced at 50
// ---------------------------------------------------------------------------

/// Registers 60 players all with the same region and position, then calls
/// `filter_players` with `limit=100`. Asserts that at most 50 results are
/// returned — proving the internal cap of 50 is enforced and that the
/// per-call cost cannot exceed O(50) result fetches regardless of bucket size.
#[test]
fn test_filter_players_page_limit_enforced() {
    let (env, client) = setup();

    for _ in 0..60 {
        let wallet = Address::generate(&env);
        client.register_player(
            &wallet,
            &dummy_vitals(&env, "Forward", "WestAfrica"),
            &dummy_hashes(&env),
        );
    }

    // Request up to 100 results — must be capped at 50 internally.
    let result = client.filter_players(
        &String::from_str(&env, "WestAfrica"),
        &String::from_str(&env, "Forward"),
        &ProgressLevel::Unverified,
        &0u32,
        &100u32,
    );

    assert!(
        result.profiles.len() <= 50,
        "filter_players must return at most 50 results; got {}",
        result.profiles.len()
    );
    assert_eq!(
        result.profiles.len(),
        50,
        "with 60 matching players and limit=100, exactly 50 should be returned"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Pagination retrieves remaining results correctly
// ---------------------------------------------------------------------------

/// Registers 60 players and paginates: first page (offset=0, limit=50) returns
/// 50, second page (offset=50, limit=50) returns 10.
/// This verifies pagination works and all 60 are reachable across pages.
#[test]
fn test_filter_players_pagination_retrieves_all() {
    let (env, client) = setup();

    for _ in 0..60 {
        let wallet = Address::generate(&env);
        client.register_player(
            &wallet,
            &dummy_vitals(&env, "Midfielder", "EastAfrica"),
            &dummy_hashes(&env),
        );
    }

    let page1 = client.filter_players(
        &String::from_str(&env, "EastAfrica"),
        &String::from_str(&env, "Midfielder"),
        &ProgressLevel::Unverified,
        &0u32,
        &50u32,
    );
    assert_eq!(page1.profiles.len(), 50, "page 1 must return 50 results");
    assert!(page1.next_cursor > 0, "page 1 must indicate more results");

    let page2 = client.filter_players(
        &String::from_str(&env, "EastAfrica"),
        &String::from_str(&env, "Midfielder"),
        &ProgressLevel::Unverified,
        &(page1.next_cursor as u32),
        &50u32,
    );
    assert_eq!(
        page2.profiles.len(),
        10,
        "page 2 must return the remaining 10 results"
    );
    assert_eq!(page2.next_cursor, 0, "page 2 must indicate no more results");

    // Total across both pages = 60.
    let total = page1.profiles.len() + page2.profiles.len();
    assert_eq!(total, 60, "total players across both pages must be 60");
}

// ---------------------------------------------------------------------------
// Test 3: filter_players CPU cost stays within budget with 50 results
// ---------------------------------------------------------------------------

/// Registers 50 players and measures `filter_players` CPU cost.
/// Budget: 15,000,000 CPU instructions (matches ci/cpu-cost-budget.md).
#[test]
fn test_filter_players_cpu_cost_at_50_results() {
    let (env, client) = setup();
    const FILTER_PLAYERS_BUDGET: u64 = 15_000_000;

    for _ in 0..50 {
        let wallet = Address::generate(&env);
        client.register_player(
            &wallet,
            &dummy_vitals(&env, "Goalkeeper", "SouthAfrica"),
            &dummy_hashes(&env),
        );
    }

    env.cost_estimate().budget().reset_default();
    let result = client.filter_players(
        &String::from_str(&env, "SouthAfrica"),
        &String::from_str(&env, "Goalkeeper"),
        &ProgressLevel::Unverified,
        &0u32,
        &50u32,
    );
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();

    println!(
        "gas_griefing: filter_players(50 results) = {cpu} cpu instructions \
         (budget {FILTER_PLAYERS_BUDGET})"
    );

    assert_eq!(result.profiles.len(), 50);
    assert!(
        cpu <= FILTER_PLAYERS_BUDGET,
        "filter_players(50 results) exceeded budget: {cpu} > {FILTER_PLAYERS_BUDGET}"
    );
}
