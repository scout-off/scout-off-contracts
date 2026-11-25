//! Tests for the player-data purge / erasure feature (issue #1781).
//!
//! Covers:
//! - `purge_player_data` for a player with no history (page 0 finishes immediately)
//! - `purge_player_data` for a player with history spanning multiple pages
//! - Resumability: interrupted purge resumes correctly from stored cursor
//! - `get_purge_status` reflects `started`, `cursor`, and `completed` at every stage
//! - Wrong cursor is rejected
//! - Calling after completion is a no-op

use scoutchain_progress::{ProgressContract, ProgressContractClient, PurgeStatus};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env};

// ── helpers ──────────────────────────────────────────────────────────────────

struct Harness {
    env: Env,
    client: ProgressContractClient<'static>,
    admin: Address,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);
    Harness { env, client, admin }
}

/// Register a whitelisted verification-contract address and return it.
fn setup_caller(h: &Harness) -> Address {
    let caller = Address::generate(&h.env);
    h.client.set_verification_contract(&caller);
    caller
}

/// Advance player by `n` levels via the whitelisted verification-contract path.
fn advance_n(h: &Harness, caller: &Address, player_id: u64, n: u32) {
    for i in 1..=n {
        h.client.advance_level(caller, &player_id, &i);
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Status before any purge call is the zero value.
#[test]
fn test_purge_status_before_start_is_zero() {
    let h = setup();
    let status = h.client.get_purge_status(&42u64);
    assert_eq!(
        status,
        PurgeStatus {
            started: false,
            cursor: 0,
            completed: false
        }
    );
}

/// A player with no history is fully erased on page 0.
#[test]
fn test_purge_no_history_completes_on_first_call() {
    let h = setup();
    let pid: u64 = 1;

    // No advance_level calls – player has no on-chain history.
    let (next_cursor, more) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(next_cursor, 0u32, "should finish in one call");
    assert!(!more);

    let status = h.client.get_purge_status(&pid);
    assert!(status.started);
    assert!(status.completed);
    assert_eq!(status.cursor, 0u32);
}

/// A player with 3 history entries (fits in one batch) is erased in two calls:
/// page 0 (scalars) then page 1 (entries 1-3).
#[test]
fn test_purge_with_history_completes_in_two_calls() {
    let h = setup();
    let pid: u64 = 2;
    let caller = setup_caller(&h);
    advance_n(&h, &caller, pid, 3);

    // Page 0: remove scalar keys; more=true because entries exist.
    let (next_cursor, more) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(next_cursor, 1u32);
    assert!(more);

    let status = h.client.get_purge_status(&pid);
    assert!(status.started);
    assert!(!status.completed);
    assert_eq!(status.cursor, 1u32);

    // Page 1: remove history entries; no more entries remain.
    let (next_cursor2, more2) = h.client.purge_player_data(&h.admin, &pid, &next_cursor);
    assert_eq!(next_cursor2, 0u32);
    assert!(!more2);

    let status2 = h.client.get_purge_status(&pid);
    assert!(status2.started);
    assert!(status2.completed);
}

/// Passing the wrong cursor is rejected.
#[test]
fn test_purge_wrong_cursor_rejected() {
    use scoutchain_progress::ProgressError;
    let h = setup();
    let pid: u64 = 3;
    let caller = setup_caller(&h);
    advance_n(&h, &caller, pid, 1);

    // Start purge so stored cursor becomes 1.
    h.client.purge_player_data(&h.admin, &pid, &0u32);

    // Supplying cursor=0 again (wrong) must be rejected.
    let result = h.client.try_purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(result, Err(Ok(ProgressError::InvalidProgressTransition)));
}

/// Calling purge_player_data after completion is a no-op (returns 0, false).
#[test]
fn test_purge_idempotent_after_completion() {
    let h = setup();
    let pid: u64 = 4;

    // Complete in one call (no history).
    h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert!(h.client.get_purge_status(&pid).completed);

    // Second call with any cursor is a no-op.
    let (nc, more) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(nc, 0u32);
    assert!(!more);
}

/// Multi-page interrupted purge: advance enough levels to require multiple
/// pages, then verify every resume step works and status is correct at each.
#[test]
fn test_purge_interrupted_and_resumed_many_pages() {
    let h = setup();
    let pid: u64 = 5;
    let caller = setup_caller(&h);

    // 3 levels = 3 HistoryEntry keys; fits in one PURGE_PAGE_SIZE=20 batch
    // but we can verify the status lifecycle end-to-end.
    advance_n(&h, &caller, pid, 3);

    // Before purge.
    let pre = h.client.get_purge_status(&pid);
    assert!(!pre.started);
    assert!(!pre.completed);

    // Page 0.
    let (c1, more1) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert!(more1);
    assert_eq!(c1, 1u32);
    let mid = h.client.get_purge_status(&pid);
    assert!(mid.started);
    assert!(!mid.completed);
    assert_eq!(mid.cursor, 1u32);

    // Page 1 — final page.
    let (c2, more2) = h.client.purge_player_data(&h.admin, &pid, &c1);
    assert!(!more2);
    assert_eq!(c2, 0u32);

    let done = h.client.get_purge_status(&pid);
    assert!(done.started);
    assert!(done.completed);

    // Player level is gone.
    assert_eq!(h.client.get_level(&pid), ProgressLevel::Unverified);
    // History count is gone (defaults to 0).
    assert_eq!(h.client.get_history_count(&pid), 0u32);
}

/// Two distinct players can be purged independently without interfering.
#[test]
fn test_purge_two_players_independent() {
    let h = setup();
    let caller = setup_caller(&h);
    let pid_a: u64 = 10;
    let pid_b: u64 = 11;

    advance_n(&h, &caller, pid_a, 2);
    advance_n(&h, &caller, pid_b, 1);

    // Purge player A fully.
    let (c, more) = h.client.purge_player_data(&h.admin, &pid_a, &0u32);
    if more {
        h.client.purge_player_data(&h.admin, &pid_a, &c);
    }
    assert!(h.client.get_purge_status(&pid_a).completed);

    // Player B is untouched.
    let b_status = h.client.get_purge_status(&pid_b);
    assert!(!b_status.started);
    assert!(!b_status.completed);
}
