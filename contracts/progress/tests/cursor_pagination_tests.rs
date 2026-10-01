//! Tests for stable cursor-based pagination (issue #800).
//!
//! These tests prove that `get_history_page_with_cursor` guarantees every
//! history entry is seen **exactly once** even when new entries are appended
//! between page fetches — unlike the plain offset-based
//! `get_progress_history_page` which can skip or duplicate entries under
//! concurrent mutation.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

fn valid_vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Forward"),
        region: String::from_str(env, "EU"),
        nationality: String::from_str(env, "FR"),
    }
}

fn one_hash(env: &Env) -> Vec<String> {
    let mut v = Vec::new(env);
    v.push_back(String::from_str(env, "bafytestcid"));
    v
}

// ── helpers ──────────────────────────────────────────────────────────────────

struct Harness {
    env: Env,
    client: ProgressContractClient<'static>,
    registration: RegistrationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);

    client.set_registration_contract(&reg_id);
    registration.set_progress_contract(&id);

    Harness { env, client, registration }
}

/// Register a player and return the assigned player ID.
fn register_player(h: &Harness) -> u64 {
    let wallet = Address::generate(&h.env);
    h.registration.register_player(&wallet, &valid_vitals(&h.env), &one_hash(&h.env)).unwrap()
}

/// Advance `player_id` by `n` levels using a whitelisted caller.
fn advance_n(h: &Harness, caller: &Address, player_id: u64, n: u32) {
    for i in 1..=n {
        h.client.advance_level(caller, &player_id, &i);
    }
}

/// Whitelist a fresh address on the *primary* (VerificationContract) path so
/// `advance_level` accepts it.
///
/// The secondary (ScoutAccessContract) path is deliberately not used: since
/// #457 it cross-calls `get_milestone_count` to validate `milestone_ref`,
/// which requires a real deployed verification contract. Pagination behaviour
/// is independent of milestone validation, so the primary path — which skips
/// that check by design — keeps this harness focused.
fn setup_whitelisted_caller(h: &Harness) -> Address {
    let caller = Address::generate(&h.env);
    h.client.set_verification_contract(&caller);
    caller
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// First call with no cursor returns page 1 and a valid next cursor.
#[test]
fn test_first_page_no_cursor() {
    let h = setup();
    let player_id = register_player(&h);
    let ver = setup_whitelisted_caller(&h);
    advance_n(&h, &ver, player_id, 3); // 3 history entries

    let (entries, next_index, snapshot) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &2u32);

    assert_eq!(
        snapshot, 3,
        "snapshot should capture all 3 existing entries"
    );
    assert_eq!(entries.len(), 2, "first page should return 2 entries");
    assert_eq!(next_index, 3, "next_index should point to entry 3");
}

/// Full walk: paging through all entries in multiple calls yields every entry
/// exactly once with no gaps.
#[test]
fn test_full_walk_no_gaps_no_duplicates() {
    let h = setup();
    let player_id = register_player(&h);
    let ver = setup_whitelisted_caller(&h);
    advance_n(&h, &ver, player_id, 3);

    let mut all_new_levels: soroban_sdk::Vec<ProgressLevel> = soroban_sdk::Vec::new(&h.env);

    // Page 1 (limit 2)
    let (p1, next1, snap1) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &2u32);
    assert_eq!(p1.len(), 2);
    for i in 0..p1.len() {
        all_new_levels.push_back(p1.get(i).unwrap().new_level);
    }

    // Page 2 (limit 2, should return 1 remaining entry)
    let (p2, next2, snap2) =
        h.client
            .get_history_page_with_cursor(&player_id, &Some(snap1), &Some(next1), &2u32);
    assert_eq!(snap2, snap1, "snapshot must not change between pages");
    assert_eq!(p2.len(), 1);
    assert_eq!(next2, 0u32, "next_index=0 signals exhaustion");
    for i in 0..p2.len() {
        all_new_levels.push_back(p2.get(i).unwrap().new_level);
    }

    // All three levels seen exactly once in order
    assert_eq!(all_new_levels.len(), 3);
    assert_eq!(
        all_new_levels.get(0).unwrap(),
        ProgressLevel::VerifiedIdentity
    );
    assert_eq!(
        all_new_levels.get(1).unwrap(),
        ProgressLevel::PerformanceMilestones
    );
    assert_eq!(all_new_levels.get(2).unwrap(), ProgressLevel::EliteTier);
}

/// Key invariant: entries appended AFTER the first page fetch are NOT visible
/// to the in-progress cursor — proving skip/duplicate freedom under concurrent
/// mutation. This is the core guarantee that plain offset pagination cannot make.
#[test]
fn test_new_entries_not_visible_to_existing_cursor() {
    let h = setup();
    let player_id = register_player(&h);
    let ver = setup_whitelisted_caller(&h);

    // Start with 2 history entries
    advance_n(&h, &ver, player_id, 2);

    // Fetch first page — snapshot locks at count=2
    let (p1, next1, snap1) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &1u32);
    assert_eq!(snap1, 2);
    assert_eq!(p1.len(), 1);
    assert_eq!(next1, 2u32);

    // NEW WRITE between pages: reset player back to Unverified, then advance again.
    // With plain offset this would shift indices and cause duplicates. With cursor
    // the snapshot_count remains 2 so the second page only sees entry 2.
    h.client
        .reset_player_level(&player_id, &ProgressLevel::Unverified);
    // That reset appended a new HistoryEntry at index 3; total is now 3.
    assert_eq!(
        h.client.get_history_count(&player_id),
        3,
        "reset should have appended a history entry"
    );

    // Resume with the cursor snapshotted at 2 — must NOT see the new entry at 3
    let (p2, next2, snap2) =
        h.client
            .get_history_page_with_cursor(&player_id, &Some(snap1), &Some(next1), &10u32);
    assert_eq!(snap2, snap1, "snapshot unchanged");
    assert_eq!(p2.len(), 1, "only entry 2 is within the snapshot window");
    assert_eq!(
        next2, 0u32,
        "cursor exhausted after seeing both snapshotted entries"
    );
}

/// Empty history returns empty vec, zero next_index, zero snapshot.
#[test]
fn test_empty_history() {
    let h = setup();
    let player_id = register_player(&h);

    let (entries, next_index, snapshot) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &10u32);

    assert_eq!(entries.len(), 0);
    assert_eq!(next_index, 0u32);
    assert_eq!(snapshot, 0u32);
}

/// Calling with an exhausted cursor (next_index = 0) returns empty + signals done.
#[test]
fn test_exhausted_cursor_returns_empty() {
    let h = setup();
    let player_id = register_player(&h);
    let ver = setup_whitelisted_caller(&h);
    advance_n(&h, &ver, player_id, 1);

    // Exhaust in one page
    let (_, next1, snap1) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &50u32);
    assert_eq!(next1, 0u32);

    // Calling again with next_index=0 should return empty
    let (entries, next2, _) =
        h.client
            .get_history_page_with_cursor(&player_id, &Some(snap1), &Some(0u32), &10u32);
    assert_eq!(entries.len(), 0);
    assert_eq!(next2, 0u32);
}

/// Limit is capped at 50; passing 100 returns at most 50 entries.
#[test]
fn test_limit_capped_at_50() {
    let h = setup();
    let player_id = register_player(&h);
    let ver = setup_whitelisted_caller(&h);
    // Only 3 entries available but we request 100
    advance_n(&h, &ver, player_id, 3);

    let (entries, _, _) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &100u32);
    assert_eq!(entries.len(), 3, "all 3 returned even though limit > count");
}

/// A cursor snapshot taken mid-history only exposes entries up to that point,
/// proving snapshot isolation when used across multiple consumers.
#[test]
fn test_snapshot_isolation_two_consumers() {
    let h = setup();
    let player_id = register_player(&h);
    let ver = setup_whitelisted_caller(&h);

    // Consumer A starts with 2 entries
    advance_n(&h, &ver, player_id, 2);
    let (_, next_a, snap_a) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &1u32);

    // New entry added before Consumer B starts
    h.client
        .reset_player_level(&player_id, &ProgressLevel::Unverified);

    // Consumer B starts fresh — sees 3 entries (snapshot_count=3)
    let (_, _, snap_b) = h
        .client
        .get_history_page_with_cursor(&player_id, &None, &None, &1u32);
    assert_eq!(snap_b, 3, "consumer B sees all 3 entries");

    // Consumer A resumes — still capped at 2, does not see entry 3
    let (p_a2, next_a2, _) =
        h.client
            .get_history_page_with_cursor(&player_id, &Some(snap_a), &Some(next_a), &10u32);
    assert_eq!(
        p_a2.len(),
        1,
        "consumer A sees only entry 2 (within snapshot)"
    );
    assert_eq!(next_a2, 0u32, "consumer A exhausted");
}

// ── #1465: overflow and validation tests ─────────────────────────────────────

/// Passing cursor_snapshot = u32::MAX must not panic; it is clamped to the
/// real count (0 for a player with no history) and returns empty.
#[test]
fn test_cursor_snapshot_umax_does_not_panic() {
    let h = setup();
    let player_id: u64 = 30;

    // No history — real count is 0. Caller-supplied u32::MAX must not trap.
    let (entries, next_index, snapshot) = h
        .client
        .get_history_page_with_cursor(&player_id, &Some(u32::MAX), &Some(1u32), &10u32);

    assert_eq!(entries.len(), 0, "no entries for a player with no history");
    assert_eq!(snapshot, 0u32, "snapshot clamped to real count (0)");
    assert_eq!(next_index, 0u32);
}

/// cursor_next_index near u32::MAX must not cause overflow when computing
/// `end = next_index + effective_limit - 1`.
#[test]
fn test_cursor_next_index_near_umax_does_not_panic() {
    let h = setup();
    let player_id: u64 = 31;
    let ver = setup_secondary_caller(&h);
    advance_n(&h, &ver, player_id, 3); // real count = 3

    // next_index far beyond snapshot_count — must exit early, not overflow.
    let (entries, next_index, _) = h.client.get_history_page_with_cursor(
        &player_id,
        &Some(3u32),
        &Some(u32::MAX - 10),
        &50u32,
    );

    assert_eq!(entries.len(), 0, "next_index beyond snapshot returns empty");
    assert_eq!(next_index, 0u32);
}

/// A caller-supplied cursor_snapshot larger than the real count is clamped.
#[test]
fn test_cursor_snapshot_larger_than_real_count_is_clamped() {
    let h = setup();
    let player_id: u64 = 32;
    let ver = setup_secondary_caller(&h);
    advance_n(&h, &ver, player_id, 2); // real count = 2

    // Pass snapshot = 1000 — must be clamped to 2, return at most 2 entries.
    let (entries, _next, snapshot) = h
        .client
        .get_history_page_with_cursor(&player_id, &Some(1000u32), &Some(1u32), &50u32);

    assert_eq!(snapshot, 2u32, "snapshot must be clamped to real count");
    assert_eq!(entries.len(), 2);
}

/// get_progress_history_page with extreme offset/limit values must not panic.
#[test]
fn test_history_page_extreme_offset_limit_no_panic() {
    let h = setup();
    let player_id: u64 = 33;
    let ver = setup_secondary_caller(&h);
    advance_n(&h, &ver, player_id, 3);

    // offset = u32::MAX — beyond count, must return empty
    let entries = h
        .client
        .get_progress_history_page(&player_id, &u32::MAX, &50u32);
    assert_eq!(entries.len(), 0);

    // limit = u32::MAX — capped at 50, must return the 3 available entries
    let entries = h
        .client
        .get_progress_history_page(&player_id, &0u32, &u32::MAX);
    assert_eq!(entries.len(), 3);
}
