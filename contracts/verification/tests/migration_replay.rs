//! Tests for issue #1038: verified cross-contract state migration replay.
//!
//! Covers `admin_seed_milestone` and `admin_seed_dispute` in the verification
//! contract, plus migration-window management.

use scoutchain_verification::{
    Milestone, MilestoneDispute, VerificationContract, VerificationContractClient,
    VerificationError,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

const VALID_CID_1: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const VALID_CID_2: &str = "QmT5NvUtoM5nWFfrQdVrFtvgfKFmG7Bze9R7WB7Wz2rFQR";
const VALID_CID_3: &str = "QmNLei78zWmzUdbeRB3CiUfAizWUrbeeZh5K1rhAQKCh51";

fn setup() -> (Env, VerificationContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin)
}

fn make_milestone(env: &Env, player_id: u64, validator: &Address, cid: &str) -> Milestone {
    Milestone {
        player_id,
        validator: validator.clone(),
        description: String::from_str(env, "Test milestone"),
        evidence_hash: String::from_str(env, cid),
        approved_at: 1_700_000_000u64,
        ledger_sequence: 1000u32,
    }
}

// ── Migration window ──────────────────────────────────────────────────────────

#[test]
fn test_verification_migration_window_lifecycle() {
    let (_env, client, _admin) = setup();
    assert!(!client.migration_window_is_open());
    client.open_migration_window();
    assert!(client.migration_window_is_open());
    client.close_migration_window();
    assert!(!client.migration_window_is_open());
}

#[test]
fn test_seed_milestone_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);
    let result = client.try_admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    assert_eq!(result, Err(Ok(VerificationError::MigrationNotActive)));
}

#[test]
fn test_seed_dispute_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    let dispute = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "unfair"),
        disputed_at: 1_700_000_000,
        resolved: false,
        upheld: false,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };
    let result = client.try_admin_seed_dispute(&1u64, &1u32, &dispute);
    assert_eq!(result, Err(Ok(VerificationError::MigrationNotActive)));
}

// ── admin_seed_milestone happy path ──────────────────────────────────────────

#[test]
fn test_seed_single_milestone_happy_path() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);

    client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);

    // Primary record readable.
    let stored = client.get_milestone(&1u64, &1u32);
    assert_eq!(stored.evidence_hash, ms.evidence_hash);

    // Counter should be 1.
    assert_eq!(client.get_milestone_count(&1u64), 1u32);
}

#[test]
fn test_seed_multiple_milestones_rebuilds_all_indexes() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);

    let ms0 = make_milestone(&env, 1, &validator, VALID_CID_1);
    let ms1 = make_milestone(&env, 1, &validator, VALID_CID_2);
    let ms2 = make_milestone(&env, 1, &validator, VALID_CID_3);

    client.admin_seed_milestone(&1u64, &1u32, &ms0, &validator);
    client.admin_seed_milestone(&1u64, &2u32, &ms1, &validator);
    client.admin_seed_milestone(&1u64, &3u32, &ms2, &validator);

    assert_eq!(client.get_milestone_count(&1u64), 3u32);
    assert_eq!(client.get_validator_milestone_count(&validator), 3u32);
    // get_validator_player_milestone_count does not exist as a public method;
    // per-player count is embedded in ValidatorMilestones.  Verify via the
    // ValidatorMilestones index that has 3 entries.
    let refs = client.get_validator_milestones(&validator);
    assert_eq!(
        refs.len(),
        3u32,
        "validator milestones index should have 3 entries"
    );
}

// ── EvidenceUsed uniqueness enforcement ──────────────────────────────────────

#[test]
fn test_duplicate_evidence_hash_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);

    let ms_a = make_milestone(&env, 1, &validator, VALID_CID_1);
    client.admin_seed_milestone(&1u64, &1u32, &ms_a, &validator);

    // Different player, same evidence hash → DuplicateEvidence.
    let ms_b = make_milestone(&env, 2, &validator, VALID_CID_1);
    let result = client.try_admin_seed_milestone(&2u64, &1u32, &ms_b, &validator);
    assert_eq!(
        result,
        Err(Ok(VerificationError::DuplicateEvidence)),
        "duplicate evidence hash must be rejected"
    );

    // Player 2 should have 0 milestones (no partial write).
    assert_eq!(client.get_milestone_count(&2u64), 0u32);
}

#[test]
fn test_same_evidence_same_slot_is_idempotent() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);

    client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    // Identical replay should succeed (no-op).
    let result = client.try_admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    assert!(result.is_ok(), "identical replay must be no-op");
    assert_eq!(client.get_milestone_count(&1u64), 1u32);
}

// ── Idempotency: counters do not inflate on replay ────────────────────────────

#[test]
fn test_counter_does_not_inflate_on_milestone_replay() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);

    client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    // Replay 5×.
    for _ in 0..5 {
        client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    }
    assert_eq!(client.get_milestone_count(&1u64), 1u32);
    assert_eq!(client.get_validator_milestone_count(&validator), 1u32);
}

// ── Conflict: different content at same index rejected ────────────────────────

#[test]
fn test_conflicting_milestone_at_same_index_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);
    client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);

    // Same index, different evidence_hash.
    let ms_conflict = make_milestone(&env, 1, &validator, VALID_CID_2);
    let result = client.try_admin_seed_milestone(&1u64, &1u32, &ms_conflict, &validator);
    assert_eq!(result, Err(Ok(VerificationError::MilestoneAlreadyExists)));
}

// ── Out-of-order index rejected ────────────────────────────────────────────────

#[test]
fn test_out_of_order_milestone_index_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms0 = make_milestone(&env, 1, &validator, VALID_CID_1);
    client.admin_seed_milestone(&1u64, &1u32, &ms0, &validator);

    // Skip index 1, try index 2.
    let ms2 = make_milestone(&env, 1, &validator, VALID_CID_3);
    let result = client.try_admin_seed_milestone(&1u64, &3u32, &ms2, &validator);
    assert_eq!(result, Err(Ok(VerificationError::MilestoneNotFound)));
    assert_eq!(client.get_milestone_count(&1u64), 1u32);
}

// ── No duplicate entries in vector indexes ────────────────────────────────────

#[test]
fn test_validator_milestones_no_duplicates_on_replay() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);

    client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    // Replay 3×.
    for _ in 0..3 {
        client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    }

    let refs = client.get_validator_milestones(&validator);
    assert_eq!(
        refs.len(),
        1u32,
        "validator milestones index must not have duplicates"
    );

    let players = client.get_validator_players(&validator);
    assert_eq!(
        players.len(),
        1u32,
        "validator players index must not have duplicates"
    );
}

// ── admin_seed_dispute happy path ─────────────────────────────────────────────

#[test]
fn test_seed_dispute_happy_path() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let dispute = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "Incorrect validation"),
        disputed_at: 1_700_000_000,
        resolved: false,
        upheld: false,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };

    client.admin_seed_dispute(&1u64, &1u32, &dispute);

    let stored = client.get_dispute(&1u64, &1u32);
    assert_eq!(stored.reason, dispute.reason);
    assert!(!stored.resolved);

    // ActiveDisputesCount should be 1.
    assert_eq!(client.get_active_disputes_count(), 1u32);
}

#[test]
fn test_seed_resolved_dispute_does_not_increment_active_count() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let dispute = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "Resolved"),
        disputed_at: 1_700_000_000,
        resolved: true,
        upheld: false,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };

    client.admin_seed_dispute(&1u64, &1u32, &dispute);
    assert_eq!(client.get_active_disputes_count(), 0u32);
}

/// Issue #1189: a seeded already-resolved dispute must not be closeable a
/// second time through either the admin path or the jury path — a spurious
/// decrement would underflow `ActiveDisputesCount` (which the seed left at 0).
#[test]
fn test_seed_resolved_dispute_cannot_be_closed_again_no_underflow() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let admin_dispute = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "Admin-path dispute resolved off-chain"),
        disputed_at: 1_700_000_000,
        resolved: true,
        upheld: true,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };
    client.admin_seed_dispute(&1u64, &1u32, &admin_dispute);

    let jury_dispute = MilestoneDispute {
        player_id: 2,
        milestone_index: 0,
        reason: String::from_str(&env, "Jury-path dispute resolved off-chain"),
        disputed_at: 1_700_000_000,
        resolved: true,
        upheld: false,
        impact_score: 100,
        jury_required: true,
        quorum: 3,
        voting_deadline: 1_700_000_500,
        votes_for: 0,
        votes_against: 0,
    };
    client.admin_seed_dispute(&2u64, &0u32, &jury_dispute);

    assert_eq!(client.get_active_disputes_count(), 0u32);

    // Admin path on an already-resolved dispute: rejected, no decrement.
    assert_eq!(
        client.try_resolve_dispute(&1u64, &1u32, &true),
        Err(Ok(VerificationError::DisputeAlreadyResolved))
    );
    // Jury path on an already-resolved dispute: rejected, no decrement.
    assert_eq!(
        client.try_tally_dispute(&2u64, &0u32),
        Err(Ok(VerificationError::DisputeAlreadyResolved))
    );

    assert_eq!(client.get_active_disputes_count(), 0u32);
}

// ── Dispute idempotency ────────────────────────────────────────────────────────

#[test]
fn test_identical_dispute_replay_is_noop() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let dispute = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "bad"),
        disputed_at: 1_700_000_000,
        resolved: false,
        upheld: false,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };

    client.admin_seed_dispute(&1u64, &1u32, &dispute);
    let result = client.try_admin_seed_dispute(&1u64, &1u32, &dispute);
    assert!(result.is_ok());
    assert_eq!(client.get_active_disputes_count(), 1u32);
}

// ── Dispute conflict rejected ──────────────────────────────────────────────────

#[test]
fn test_conflicting_dispute_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let dispute = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "original"),
        disputed_at: 1_700_000_000,
        resolved: false,
        upheld: false,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };
    client.admin_seed_dispute(&1u64, &1u32, &dispute);

    let mut different = dispute.clone();
    different.reason = String::from_str(&env, "different reason");
    let result = client.try_admin_seed_dispute(&1u64, &1u32, &different);
    assert_eq!(result, Err(Ok(VerificationError::DisputeAlreadyExists)));
}

// ── PlayerDisputes index populated ────────────────────────────────────────────

#[test]
fn test_seed_dispute_populates_player_disputes_index() {
    let (env, client, _admin) = setup();
    client.open_migration_window();

    let d0 = MilestoneDispute {
        player_id: 1,
        milestone_index: 1,
        reason: String::from_str(&env, "first"),
        disputed_at: 1_700_000_000,
        resolved: false,
        upheld: false,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };
    let d1 = MilestoneDispute {
        player_id: 1,
        milestone_index: 2,
        reason: String::from_str(&env, "second"),
        disputed_at: 1_700_000_001,
        resolved: true,
        upheld: true,
        impact_score: 0,
        jury_required: false,
        quorum: 0,
        voting_deadline: 0,
        votes_for: 0,
        votes_against: 0,
    };

    client.admin_seed_dispute(&1u64, &1u32, &d0);
    client.admin_seed_dispute(&1u64, &2u32, &d1);

    let indices = client.get_player_disputes(&1u64, &0u32, &50u32);
    assert_eq!(indices.len(), 2u32);
    assert_eq!(client.get_active_disputes_count(), 1u32); // only d0 is open
}

// ── Security: seeding after close fails ──────────────────────────────────────

#[test]
fn test_verification_seed_rejected_after_window_close() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let validator = Address::generate(&env);
    let ms = make_milestone(&env, 1, &validator, VALID_CID_1);
    client.admin_seed_milestone(&1u64, &1u32, &ms, &validator);
    client.close_migration_window();

    let ms2 = make_milestone(&env, 1, &validator, VALID_CID_2);
    let result = client.try_admin_seed_milestone(&1u64, &2u32, &ms2, &validator);
    assert_eq!(result, Err(Ok(VerificationError::MigrationNotActive)));
}
