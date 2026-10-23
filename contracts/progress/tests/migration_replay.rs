//! Tests for issue #1038: verified cross-contract state migration replay.
//!
//! Covers `admin_seed_history`, `open_migration_window`, and
//! `close_migration_window` in the progress contract.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressEntry, ProgressError};
use scoutchain_shared_types::testutils::count_events;
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Symbol};

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup() -> (Env, ProgressContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &id);
    (env, client, admin)
}

fn level_at(index: u32) -> ProgressLevel {
    match index {
        0 => ProgressLevel::Unverified,
        1 => ProgressLevel::VerifiedIdentity,
        2 => ProgressLevel::PerformanceMilestones,
        _ => ProgressLevel::EliteTier,
    }
}

/// Entries chain: entry `index` moves `level_at(index - 1)` -> `level_at(index)`.
fn make_entry(env: &Env, player_id: u64, index: u32) -> ProgressEntry {
    ProgressEntry {
        player_id,
        old_level: level_at(index.saturating_sub(1)),
        new_level: level_at(index),
        updated_by: Address::generate(env),
        updated_at: 1_700_000_000 + index as u64 * 100,
        milestone_ref: index,
        ledger_sequence: 1000 + index,
    }
}

// ── 1. Happy-path single entry ────────────────────────────────────────────────

#[test]
fn test_seed_single_entry_happy_path() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 42u64;
    let entry = make_entry(&env, player_id, 1);

    let result = client.try_admin_seed_history(&player_id, &1u32, &entry, &None);
    assert!(result.is_ok(), "single entry seed should succeed");

    assert_eq!(client.get_history_count(&player_id), 1u32);
    let stored = client.get_history_entry(&player_id, &1u32);
    assert_eq!(stored.player_id, entry.player_id);
    assert_eq!(stored.milestone_ref, entry.milestone_ref);
}

// ── 2. Multi-entry with root verification ────────────────────────────────────

#[test]
fn test_seed_multi_entry_with_root_verification() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 1u64;
    let e1 = make_entry(&env, player_id, 1);
    let e2 = make_entry(&env, player_id, 2);
    let e3 = make_entry(&env, player_id, 3);

    // Seed first two without root check.
    client.admin_seed_history(&player_id, &1u32, &e1, &None);
    client.admin_seed_history(&player_id, &2u32, &e2, &None);
    // Final seed with expected_root — but we get the root from
    // the intermediate state first, then supply it as the expected root
    // for the final call on a second contract registered in the SAME env.
    client.admin_seed_history(&player_id, &3u32, &e3, &None);
    let root = client.get_progress_root(&player_id);

    // Register a second contract in the SAME env (no cross-env object issues).
    let admin2 = Address::generate(&env);
    let id2 = env.register(ProgressContract, (admin2.clone(),));
    let client2 = ProgressContractClient::new(&env, &id2);
    client2.open_migration_window();

    client2.admin_seed_history(&player_id, &1u32, &e1, &None);
    client2.admin_seed_history(&player_id, &2u32, &e2, &None);
    // Final call with expected_root.
    let result = client2.try_admin_seed_history(&player_id, &3u32, &e3, &Some(root.clone()));
    assert!(result.is_ok(), "correct expected_root must be accepted");

    let root2 = client2.get_progress_root(&player_id);
    assert_eq!(root, root2, "deterministic root across two replays");
}

// ── 3. Wrong expected_root rejected ──────────────────────────────────────────

#[test]
fn test_wrong_expected_root_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 5u64;
    let entry = make_entry(&env, player_id, 1);
    let wrong_root: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);

    let result = client.try_admin_seed_history(&player_id, &1u32, &entry, &Some(wrong_root));
    assert_eq!(
        result,
        Err(Ok(ProgressError::MerkleRootMismatch)),
        "wrong expected_root must be rejected"
    );
    // Entry rolled back — counter stays 0.
    assert_eq!(client.get_history_count(&player_id), 0u32);
}

// ── 4. Altered entry produces different root ──────────────────────────────────

#[test]
fn test_altered_entry_causes_root_mismatch() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 7u64;
    let entry = make_entry(&env, player_id, 1);
    client.admin_seed_history(&player_id, &1u32, &entry, &None);
    let correct_root = client.get_progress_root(&player_id);

    // Build a second contract in the SAME env with an altered entry.
    let admin2 = Address::generate(&env);
    let id2 = env.register(ProgressContract, (admin2.clone(),));
    let client2 = ProgressContractClient::new(&env, &id2);
    client2.open_migration_window();

    let mut altered = make_entry(&env, player_id, 1);
    altered.milestone_ref = 999;
    client2.admin_seed_history(&player_id, &1u32, &altered, &None);
    let altered_root = client2.get_progress_root(&player_id);

    assert_ne!(
        correct_root, altered_root,
        "altering an entry must change the root"
    );

    // Third contract: seeding the altered entry with the ORIGINAL root must fail.
    let admin3 = Address::generate(&env);
    let id3 = env.register(ProgressContract, (admin3.clone(),));
    let client3 = ProgressContractClient::new(&env, &id3);
    client3.open_migration_window();

    let mut altered3 = make_entry(&env, player_id, 1);
    altered3.milestone_ref = 999;
    let result = client3.try_admin_seed_history(&player_id, &1u32, &altered3, &Some(correct_root));
    assert_eq!(result, Err(Ok(ProgressError::MerkleRootMismatch)));
}

// ── 5. Out-of-order index rejected ───────────────────────────────────────────

#[test]
fn test_out_of_order_index_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 10u64;
    let e1 = make_entry(&env, player_id, 1);
    let e3 = make_entry(&env, player_id, 3); // gap

    client.admin_seed_history(&player_id, &1u32, &e1, &None);
    let result = client.try_admin_seed_history(&player_id, &3u32, &e3, &None);
    assert_eq!(result, Err(Ok(ProgressError::InvalidHistoryIndex)));
    assert_eq!(client.get_history_count(&player_id), 1u32);
}

// ── 6. Zero index rejected ────────────────────────────────────────────────────

#[test]
fn test_zero_index_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let entry = make_entry(&env, 11u64, 0);
    let result = client.try_admin_seed_history(&11u64, &0u32, &entry, &None);
    assert_eq!(result, Err(Ok(ProgressError::InvalidHistoryIndex)));
}

// ── 7. Idempotency: identical replay is no-op ─────────────────────────────────

#[test]
fn test_identical_replay_is_noop() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 20u64;
    let entry = make_entry(&env, player_id, 1);

    client.admin_seed_history(&player_id, &1u32, &entry, &None);
    let root_after_first = client.get_progress_root(&player_id);

    let result = client.try_admin_seed_history(&player_id, &1u32, &entry, &None);
    assert!(result.is_ok(), "identical replay must be a no-op");

    assert_eq!(client.get_history_count(&player_id), 1u32);
    assert_eq!(client.get_progress_root(&player_id), root_after_first);
}

// ── 8. Counter does not double-increment ─────────────────────────────────────

#[test]
fn test_counter_does_not_double_increment() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 21u64;
    let entry = make_entry(&env, player_id, 1);
    client.admin_seed_history(&player_id, &1u32, &entry, &None);
    assert_eq!(client.get_history_count(&player_id), 1u32);

    for _ in 0..3 {
        let r = client.try_admin_seed_history(&player_id, &1u32, &entry, &None);
        assert!(r.is_ok());
    }
    assert_eq!(client.get_history_count(&player_id), 1u32);
}

// ── 9. Conflicting content at same index rejected ─────────────────────────────

#[test]
fn test_conflicting_entry_at_same_index_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 22u64;
    let entry = make_entry(&env, player_id, 1);
    client.admin_seed_history(&player_id, &1u32, &entry, &None);

    let mut different = make_entry(&env, player_id, 1);
    different.milestone_ref = 42;
    let result = client.try_admin_seed_history(&player_id, &1u32, &different, &None);
    assert_eq!(result, Err(Ok(ProgressError::HistoryAlreadyExists)));

    let stored = client.get_history_entry(&player_id, &1u32);
    assert_eq!(stored.milestone_ref, entry.milestone_ref);
}

// ── 10. Partial replay resumes safely ────────────────────────────────────────

#[test]
fn test_partial_replay_resumes_safely() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 30u64;

    // Session 1: seed 1–3.
    for i in 1u32..=3 {
        let e = make_entry(&env, player_id, i);
        client.admin_seed_history(&player_id, &i, &e, &None);
    }
    assert_eq!(client.get_history_count(&player_id), 3u32);

    // Session 2: seed 4–5.
    for i in 4u32..=5 {
        let e = make_entry(&env, player_id, i);
        client.admin_seed_history(&player_id, &i, &e, &None);
    }
    assert_eq!(client.get_history_count(&player_id), 5u32);
    let root = client.get_progress_root(&player_id);
    assert_ne!(root, BytesN::from_array(&env, &[0u8; 32]));
}

// ── 11. Non-admin rejected ────────────────────────────────────────────────────

#[test]
fn test_non_admin_cannot_seed_history() {
    // Verify that without admin auth, the seed call fails.
    // We use a fresh env with no mocked auths; require_auth on the stored admin
    // will panic, which try_ surfaces as Err.
    let env = Env::default();
    // NO mock_all_auths.
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &id);

    // Open migration window with admin auth.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &id,
            fn_name: "open_migration_window",
            args: soroban_sdk::vec![&env],
            sub_invokes: &[],
        },
    }]);
    client.open_migration_window();

    // Seed with NO mock auth — require_auth on admin fails.
    let entry = make_entry(&env, 1u64, 1);
    // (no mock_auths set here — any require_auth call will trap)
    let result = client.try_admin_seed_history(&1u64, &1u32, &entry, &None);
    assert!(result.is_err(), "unauthenticated call must fail");
}

// ── 12. Window closed: seed rejected ─────────────────────────────────────────

#[test]
fn test_seed_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    // Window is NOT opened.
    let entry = make_entry(&env, 1u64, 1);
    let result = client.try_admin_seed_history(&1u64, &1u32, &entry, &None);
    assert_eq!(result, Err(Ok(ProgressError::MigrationNotActive)));
}

// ── 13. Window lifecycle ──────────────────────────────────────────────────────

#[test]
fn test_migration_window_lifecycle() {
    let (_env, client, _admin) = setup();
    assert!(!client.migration_window_is_open(), "initially closed");
    client.open_migration_window();
    assert!(client.migration_window_is_open(), "open after open call");
    client.close_migration_window();
    assert!(
        !client.migration_window_is_open(),
        "closed after close call"
    );
}

// ── 14. Seeding after close fails ────────────────────────────────────────────

#[test]
fn test_seed_rejected_after_close() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 50u64;
    let entry = make_entry(&env, player_id, 1);
    client.admin_seed_history(&player_id, &1u32, &entry, &None);

    client.close_migration_window();

    let entry2 = make_entry(&env, player_id, 2);
    let result = client.try_admin_seed_history(&player_id, &2u32, &entry2, &None);
    assert_eq!(result, Err(Ok(ProgressError::MigrationNotActive)));
    assert_eq!(client.get_history_count(&player_id), 1u32);
}

// ── 15. Seeded root verifiable via Merkle proof ───────────────────────────────

#[test]
fn test_seeded_root_verifiable_via_proof() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 60u64;
    let mut entries = Vec::new();
    for i in 1u32..=3 {
        let e = make_entry(&env, player_id, i);
        entries.push(e.clone());
        client.admin_seed_history(&player_id, &i, &e, &None);
    }

    for (i, entry) in entries.iter().enumerate() {
        let idx = (i + 1) as u32;
        let proof = client.get_history_proof(&player_id, &idx);
        let verified = client.verify_history_proof(&player_id, &entry.clone(), &proof);
        assert!(
            verified,
            "seeded entry {} must be verifiable via Merkle proof",
            idx
        );
    }
}

// ── 16. Idempotent replay with root check on already-committed history ────────

#[test]
fn test_idempotent_replay_with_root_check_on_existing() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let player_id = 70u64;
    let entry = make_entry(&env, player_id, 1);
    client.admin_seed_history(&player_id, &1u32, &entry, &None);
    let root = client.get_progress_root(&player_id);

    // Replay with correct root — no-op, passes.
    let r = client.try_admin_seed_history(&player_id, &1u32, &entry, &Some(root.clone()));
    assert!(
        r.is_ok(),
        "idempotent replay with correct root must succeed"
    );

    // Replay with wrong root — rejected.
    let wrong: BytesN<32> = BytesN::from_array(&env, &[0xffu8; 32]);
    let r2 = client.try_admin_seed_history(&player_id, &1u32, &entry, &Some(wrong));
    assert_eq!(r2, Err(Ok(ProgressError::MerkleRootMismatch)));
}

// ── 17. Reopening after close fails with MigrationWindowSealed ──────────────────
// Issue #1410: Migration window must be one-time only.

#[test]
fn test_reopening_after_close_fails() {
    let (_env, client, _admin) = setup();
    client.open_migration_window();
    assert!(client.health().migration_window_open);

    client.close_migration_window();
    assert!(!client.health().migration_window_open);

    let res = client.try_open_migration_window();
    assert_eq!(res, Err(Ok(ProgressError::MigrationWindowSealed)));
}

#[test]
fn test_migration_window_events_emitted() {
    let (env, client, _admin) = setup();

    client.open_migration_window();
    assert_eq!(
        count_events(&env, "migration_window_opened", Some(&client.address)),
        1,
        "migration_window_opened event must be emitted"
    );

    client.close_migration_window();
    assert_eq!(
        count_events(&env, "migration_window_closed", Some(&client.address)),
        1,
        "migration_window_closed event must be emitted"
    );
}

// ── #1617: seeded-entry validation ───────────────────────────────────────────

#[test]
fn test_seed_player_mismatch_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let entry = make_entry(&env, 99, 1);
    let r = client.try_admin_seed_history(&42u64, &1u32, &entry, &None);
    assert_eq!(r, Err(Ok(ProgressError::SeedPlayerMismatch)));
    assert_eq!(client.get_history_count(&42u64), 0u32);
}

#[test]
fn test_seed_level_discontinuity_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let mut e = make_entry(&env, 42, 1);
    e.new_level = ProgressLevel::EliteTier;
    client.admin_seed_history(&42u64, &1u32, &e, &None);
    // Previous new_level is EliteTier; this entry claims to start at Unverified.
    let mut e2 = make_entry(&env, 42, 2);
    e2.old_level = ProgressLevel::Unverified;
    let r = client.try_admin_seed_history(&42u64, &2u32, &e2, &None);
    assert_eq!(r, Err(Ok(ProgressError::SeedLevelDiscontinuity)));
    // First entry must start at Unverified.
    let mut first = make_entry(&env, 7, 1);
    first.old_level = ProgressLevel::EliteTier;
    let r = client.try_admin_seed_history(&7u64, &1u32, &first, &None);
    assert_eq!(r, Err(Ok(ProgressError::SeedLevelDiscontinuity)));
}

#[test]
fn test_seed_reset_may_jump_down_but_must_chain_from_old_level() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    client.admin_seed_history(&42u64, &1u32, &make_entry(&env, 42, 1), &None);
    // Reset (milestone_ref == 0): VerifiedIdentity -> Unverified is allowed.
    let mut reset = make_entry(&env, 42, 2);
    reset.old_level = ProgressLevel::VerifiedIdentity;
    reset.new_level = ProgressLevel::Unverified;
    reset.milestone_ref = 0;
    assert!(client
        .try_admin_seed_history(&42u64, &2u32, &reset, &None)
        .is_ok());
}

#[test]
fn test_seed_timestamp_regression_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    client.admin_seed_history(&42u64, &1u32, &make_entry(&env, 42, 1), &None);
    let mut e2 = make_entry(&env, 42, 2);
    e2.updated_at = 1;
    let r = client.try_admin_seed_history(&42u64, &2u32, &e2, &None);
    assert_eq!(r, Err(Ok(ProgressError::SeedTimestampRegression)));
}
