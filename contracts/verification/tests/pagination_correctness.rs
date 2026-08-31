//! Pagination correctness tests for `get_validator_milestones_page_v2` and
//! `get_validator_players_page` (issue #619).
//!
//! Acceptance-criteria coverage (mirrors pagination_correctness.rs in
//! scout_access/tests, but applied to the verification contract):
//!
//! 1. Empty pages past the end return `entries = []` and the correct `total`.
//! 2. Exact boundary: `offset + limit == total` returns the last chunk only.
//! 3. `limit` is clamped at 50 — requesting more never yields more than 50.
//! 4. Walking all pages with `limit = 3` covers every milestone / player
//!    exactly once in insertion order.
//! 5. A validator with no approvals returns `total = 0`.

use scoutchain_verification::{MilestoneRef, VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

/// Register a validator with a default credential and no specializations.
fn register_validator(env: &Env, client: &VerificationContractClient<'static>) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, "UEFA-B-License"),
        &String::from_str(env, "Default Academy"),
        &Vec::new(env),
    );
    wallet
}

/// Generate a unique, valid CIDv0 string (exactly 46 chars, base58btc charset)
/// for each `seed` value.  Two-character suffix taken from the base58 alphabet
/// so the hash is always valid and collision-free within a test.
fn cid_for(env: &Env, seed: u64) -> String {
    const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // 44-char base — CIDv0 requires exactly 46 chars (44 + 2-char seed suffix).
    let base = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygp";
    let c1 = B58[((seed / 58) % 58) as usize] as char;
    let c2 = B58[(seed % 58) as usize] as char;
    let s = std::format!("{base}{c1}{c2}");
    debug_assert_eq!(s.len(), 46, "CIDv0 must be exactly 46 chars");
    String::from_str(env, &s)
}

/// Approve one milestone for `player_id` by `validator`, using a seed-derived
/// unique evidence hash.  Progress contract is intentionally left unwired;
/// `approve_milestone` still succeeds and records the milestone (it only
/// emits a diagnostic event instead of calling advance_level).
fn approve(
    env: &Env,
    client: &VerificationContractClient<'static>,
    validator: &Address,
    player_id: u64,
    seed: u64,
) {
    client.approve_milestone(
        validator,
        &player_id,
        &String::from_str(env, "scored a goal"),
        &cid_for(env, seed),
        &None,
    );
}

// ── get_validator_milestones_page_v2 tests ───────────────────────────────────

/// Empty page past the end returns entries=[] and correct total.
#[test]
fn test_milestones_empty_page_past_end() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 4 milestones for 4 distinct players (1 per player to stay within
    // the 5-per-(validator, player) cap and avoid duplicate-evidence rejection).
    for seed in 0u64..4 {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    // offset=4 == total → empty
    let page = client.get_validator_milestones_page_v2(&validator, &4u32, &10u32);
    assert_eq!(page.total, 4);
    assert_eq!(page.entries.len(), 0, "offset past end must return empty entries");

    // offset > total
    let page_far = client.get_validator_milestones_page_v2(&validator, &100u32, &10u32);
    assert_eq!(page_far.total, 4);
    assert_eq!(page_far.entries.len(), 0);
}

/// Exact boundary: offset + limit == total returns the last chunk only.
#[test]
fn test_milestones_exact_boundary() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 6 milestones — one per player to avoid per-(validator,player) cap.
    for seed in 0u64..6 {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    // offset=4, limit=2 → entries at indices 4 and 5 (the last two)
    let page = client.get_validator_milestones_page_v2(&validator, &4u32, &2u32);
    assert_eq!(page.total, 6);
    assert_eq!(
        page.entries.len(),
        2,
        "boundary page should contain exactly 2 entries"
    );
    // Entry at index 4 was approved for player 5 (seed=4), milestone_index=1
    let e0 = page.entries.get(0).unwrap();
    assert_eq!(e0.player_id, 5u64);
    let e1 = page.entries.get(1).unwrap();
    assert_eq!(e1.player_id, 6u64);
}

/// Limit is clamped at 50.
#[test]
fn test_milestones_limit_clamped_at_50() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 55 milestones across different players (one per player to stay
    // within the per-(validator, player) limit of 5).
    for seed in 0u64..55 {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    let page = client.get_validator_milestones_page_v2(&validator, &0u32, &200u32);
    assert_eq!(page.total, 55);
    assert!(
        page.entries.len() <= 50,
        "entries must never exceed 50-entry cap (got {})",
        page.entries.len()
    );
    assert_eq!(
        page.entries.len(),
        50,
        "first 50 entries returned for offset=0 with limit=200"
    );
}

/// Walking all pages with limit=3 covers every milestone exactly once.
#[test]
fn test_milestones_walk_all_pages() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);
    const TOTAL: u64 = 10;

    // Approve milestones across different players (one per player).
    for seed in 0u64..TOTAL {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    let mut seen: soroban_sdk::Vec<MilestoneRef> = soroban_sdk::Vec::new(&env);
    let mut offset = 0u32;
    let limit = 3u32;

    loop {
        let page = client.get_validator_milestones_page_v2(&validator, &offset, &limit);
        assert_eq!(
            page.total,
            TOTAL as u32,
            "total must be consistent across pages"
        );
        if page.entries.is_empty() {
            break;
        }
        for i in 0..page.entries.len() {
            seen.push_back(page.entries.get(i).unwrap());
        }
        offset += limit;
    }

    assert_eq!(
        seen.len(),
        TOTAL as u32,
        "all {TOTAL} milestone refs must be seen exactly once"
    );
}

/// Validator with no approvals returns total=0 and empty entries.
#[test]
fn test_milestones_no_approvals_returns_empty() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    let page = client.get_validator_milestones_page_v2(&validator, &0u32, &10u32);
    assert_eq!(page.total, 0);
    assert_eq!(page.entries.len(), 0);
}

/// limit=0 returns empty entries but the correct total.
#[test]
fn test_milestones_zero_limit_returns_empty_with_total() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    approve(&env, &client, &validator, 1, 0);
    approve(&env, &client, &validator, 2, 1);

    let page = client.get_validator_milestones_page_v2(&validator, &0u32, &0u32);
    assert_eq!(page.total, 2, "total still reflects 2 milestones");
    assert_eq!(page.entries.len(), 0, "limit=0 returns zero entries");
}

// ── get_validator_players_page tests ─────────────────────────────────────────

/// Empty page past the end of the player list returns entries=[] and total.
#[test]
fn test_players_empty_page_past_end() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve milestones for 4 distinct players
    for seed in 0u64..4 {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    let page = client.get_validator_players_page(&validator, &4u32, &10u32);
    assert_eq!(page.total, 4);
    assert_eq!(page.entries.len(), 0, "offset past end must return empty entries");

    let page_far = client.get_validator_players_page(&validator, &100u32, &10u32);
    assert_eq!(page_far.total, 4);
    assert_eq!(page_far.entries.len(), 0);
}

/// Exact boundary: offset + limit == total returns the last chunk only.
#[test]
fn test_players_exact_boundary() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    for seed in 0u64..6 {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    let page = client.get_validator_players_page(&validator, &4u32, &2u32);
    assert_eq!(page.total, 6);
    assert_eq!(page.entries.len(), 2);
    // Players appear in first-approval order: IDs 5 and 6 are at indices 4 and 5
    assert_eq!(page.entries.get(0).unwrap(), 5u64);
    assert_eq!(page.entries.get(1).unwrap(), 6u64);
}

/// Limit is clamped at 50.
#[test]
fn test_players_limit_clamped_at_50() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    for seed in 0u64..55 {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    let page = client.get_validator_players_page(&validator, &0u32, &200u32);
    assert_eq!(page.total, 55);
    assert!(
        page.entries.len() <= 50,
        "entries must never exceed 50-entry cap (got {})",
        page.entries.len()
    );
    assert_eq!(page.entries.len(), 50);
}

/// Walking all pages with limit=4 covers every player exactly once.
#[test]
fn test_players_walk_all_pages() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);
    const TOTAL: u64 = 13;

    for seed in 0u64..TOTAL {
        approve(&env, &client, &validator, seed + 1, seed);
    }

    let mut seen: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&env);
    let mut offset = 0u32;
    let limit = 4u32;

    loop {
        let page = client.get_validator_players_page(&validator, &offset, &limit);
        assert_eq!(
            page.total,
            TOTAL as u32,
            "total must be consistent across pages"
        );
        if page.entries.is_empty() {
            break;
        }
        for i in 0..page.entries.len() {
            seen.push_back(page.entries.get(i).unwrap());
        }
        offset += limit;
    }

    assert_eq!(
        seen.len(),
        TOTAL as u32,
        "all {TOTAL} players must be seen exactly once"
    );
    // Verify insertion order: player IDs 1 through TOTAL in sequence.
    for i in 0..seen.len() {
        assert_eq!(
            seen.get(i).unwrap(),
            (i + 1) as u64,
            "player IDs must appear in first-approval order (oldest first)"
        );
    }
}

/// Validator with no approvals returns total=0.
#[test]
fn test_players_no_approvals_returns_empty() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    let page = client.get_validator_players_page(&validator, &0u32, &10u32);
    assert_eq!(page.total, 0);
    assert_eq!(page.entries.len(), 0);
}

/// Multiple approvals for the same player are de-duplicated — the player
/// appears at most once in the players page.
#[test]
fn test_players_deduplication() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 3 milestones for player 1 and 2 for player 2 (within the 5-per-
    // (validator, player) cap).
    approve(&env, &client, &validator, 1, 1);
    approve(&env, &client, &validator, 1, 2);
    approve(&env, &client, &validator, 1, 3);
    approve(&env, &client, &validator, 2, 4);
    approve(&env, &client, &validator, 2, 5);

    let page = client.get_validator_players_page(&validator, &0u32, &10u32);
    // Player 1 and player 2 — each once despite multiple approvals.
    assert_eq!(page.total, 2, "each distinct player appears exactly once");
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries.get(0).unwrap(), 1u64);
    assert_eq!(page.entries.get(1).unwrap(), 2u64);
}
