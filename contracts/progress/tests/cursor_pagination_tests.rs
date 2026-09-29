//! Tests for stable cursor-based pagination (issue #800).
//!
//! These tests prove that `get_history_page_with_cursor` guarantees every
//! history entry is seen **exactly once** even when new entries are appended
//! between page fetches — unlike the plain offset-based
//! `get_progress_history_page` which can skip or duplicate entries under
//! concurrent mutation.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env};

// ── helpers ──────────────────────────────────────────────────────────────────

struct Harness {
    env: Env,
    client: ProgressContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);
    Harness { env, client }
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
    let player_id: u64 = 1;
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
    let player_id: u64 = 2;
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
    let player_id: u64 = 10;
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
    let player_id: u64 = 99;

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
    let player_id: u64 = 5;
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
    let player_id: u64 = 7;
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
    let player_id: u64 = 20;
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
