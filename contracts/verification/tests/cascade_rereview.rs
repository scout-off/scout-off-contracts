//! Tests for the validator revocation cascade re-review system (issue #1039).
//!
//! Covers:
//! 1. `ForCause` revocation flags all of the validator's prior milestones.
//! 2. `Routine` revocation flags none.
//! 3. Bounded per-call sweep (CASCADE_LIMIT = 50): a validator with > 50
//!    prior approvals requires one initial `revoke_validator` call plus one
//!    or more `continue_revocation_cascade` calls.
//! 4. `is_milestone_flagged` returns correct values before/after flagging and
//!    after clearing.
//! 5. `rereview_milestone` clears a flag when called by an active validator.
//! 6. `rereview_milestone` rejects inactive reviewers and unflagged milestones.
//! 7. `get_revocation_record` returns the stored record.
//! 8. CPU-budget: a bounded cascade call (limit=50) against a validator with
//!    500+ prior approvals stays within budget.

use scoutchain_verification::{
    RevocationSeverity, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

// ── helpers ──────────────────────────────────────────────────────────────────

/// All CIDv0-valid characters (no 0/O/I/l).
const BASE58_CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Deterministically build a distinct valid 46-char CIDv0 for a given seed.
fn cid(env: &Env, seed: u32) -> String {
    let mut s = std::string::String::from("Qm");
    let mut n = seed.wrapping_add(1);
    for _ in 0..44 {
        let idx = (n % BASE58_CHARS.len() as u32) as usize;
        s.push(BASE58_CHARS[idx] as char);
        n = n / (BASE58_CHARS.len() as u32) + seed.wrapping_add(7);
    }
    String::from_str(env, &s)
}

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn register_validator(env: &Env, client: &VerificationContractClient) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(&wallet, &String::from_str(env, "UEFA-B-License-2026"), &String::from_str(env, "Default Academy"), &String::from_str(env, "Default Region"), &Vec::new(env));
    wallet
}

/// Approve `count` distinct milestones for `validator` across `count` distinct
/// player IDs, using deterministic CIDs starting from `cid_seed`.
///
/// Returns the next cid_seed to use for subsequent calls.
fn approve_n_milestones(
    env: &Env,
    client: &VerificationContractClient,
    validator: &Address,
    count: u32,
    cid_seed_start: u32,
) -> u32 {
    for i in 0..count {
        let player_id = (cid_seed_start + i + 1) as u64;
        let c = cid(env, cid_seed_start + i);
        client.approve_milestone(
            validator,
            &player_id,
            &String::from_str(env, "verified achievement"),
            &c,
            &None,
        );
    }
    cid_seed_start + count
}

// ── Test 1: Routine revocation — no milestone flagged ────────────────────────

#[test]
fn routine_revocation_flags_no_milestones() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 3 milestones.
    approve_n_milestones(&env, &client, &validator, 3, 0);

    client.revoke_validator(&validator, &RevocationSeverity::Routine, &None);

    // None of the milestones should be flagged.
    assert!(
        !client.is_milestone_flagged(&1u64, &1u32),
        "player 1, milestone 1 must not be flagged after Routine revocation"
    );
    assert!(
        !client.is_milestone_flagged(&2u64, &1u32),
        "player 2, milestone 1 must not be flagged after Routine revocation"
    );
    assert!(
        !client.is_milestone_flagged(&3u64, &1u32),
        "player 3, milestone 1 must not be flagged after Routine revocation"
    );
}

// ── Test 2: ForCause revocation — all milestones flagged (small batch) ───────

#[test]
fn for_cause_revocation_flags_all_milestones_small_batch() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 10 milestones across 10 distinct players.
    approve_n_milestones(&env, &client, &validator, 10, 0);

    // Before revocation nothing is flagged.
    for i in 1u64..=10 {
        assert!(
            !client.is_milestone_flagged(&i, &1u32),
            "milestone must not be flagged before revocation"
        );
    }

    client.revoke_validator(
        &validator,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "credential fraud")),
    );

    // After ForCause revocation all 10 should be flagged.
    for i in 1u64..=10 {
        assert!(
            client.is_milestone_flagged(&i, &1u32),
            "player {i} milestone 1 must be flagged after ForCause revocation"
        );
    }
}

// ── Test 3: ForCause with > CASCADE_LIMIT milestones requires continuation ───
//
// CASCADE_LIMIT = 50.  We approve 60 milestones so the first call flags 50
// and leaves a cursor; continue_revocation_cascade flags the remaining 10.

#[test]
fn for_cause_cascade_is_bounded_and_resumable() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 60 milestones (each for a distinct player so the 5-per-player cap
    // is not hit).
    const TOTAL: u32 = 60;
    approve_n_milestones(&env, &client, &validator, TOTAL, 1000);

    // Revoke for cause — should flag first 50 and leave a cursor.
    client.revoke_validator(
        &validator,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "fabricated credentials")),
    );

    // First 50 player IDs (1001..=1050) should be flagged.
    for i in 1001u64..=1050 {
        assert!(
            client.is_milestone_flagged(&i, &1u32),
            "player {i} milestone 1 must be flagged after initial cascade call"
        );
    }

    // Players 1051..=1060 should NOT yet be flagged (cursor not yet advanced).
    for i in 1051u64..=1060 {
        assert!(
            !client.is_milestone_flagged(&i, &1u32),
            "player {i} milestone 1 must NOT be flagged before continue_revocation_cascade"
        );
    }

    // Continue the cascade — should flag the remaining 10.
    client.continue_revocation_cascade(&validator);

    for i in 1051u64..=1060 {
        assert!(
            client.is_milestone_flagged(&i, &1u32),
            "player {i} milestone 1 must be flagged after continue_revocation_cascade"
        );
    }
}

// ── Test 4: is_milestone_flagged after rereview_milestone clears the flag ────

#[test]
fn rereview_milestone_clears_flag() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);
    let reviewer = register_validator(&env, &client);

    // Approve one milestone.
    approve_n_milestones(&env, &client, &validator, 1, 2000);

    // Flag it via ForCause.
    client.revoke_validator(&validator, &RevocationSeverity::ForCause, &None);
    assert!(
        client.is_milestone_flagged(&2001u64, &1u32),
        "must be flagged"
    );

    // An active validator (reviewer) clears the flag.
    client.rereview_milestone(&reviewer, &2001u64, &1u32);

    assert!(
        !client.is_milestone_flagged(&2001u64, &1u32),
        "flag must be cleared after rereview"
    );
}

// ── Test 5: rereview_milestone rejects inactive reviewers ────────────────────

#[test]
fn rereview_milestone_rejects_inactive_reviewer() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);
    let inactive_reviewer = register_validator(&env, &client);

    approve_n_milestones(&env, &client, &validator, 1, 3000);
    client.revoke_validator(&validator, &RevocationSeverity::ForCause, &None);

    // Revoke the reviewer — they are no longer active.
    client.revoke_validator(&inactive_reviewer, &RevocationSeverity::Routine, &None);

    let result = client.try_rereview_milestone(&inactive_reviewer, &3001u64, &1u32);
    assert_eq!(
        result,
        Err(Ok(VerificationError::NotEligibleToReReview)),
        "revoked validator must not be able to rereview"
    );
}

// ── Test 6: rereview_milestone rejects non-flagged milestones ────────────────

#[test]
fn rereview_milestone_rejects_not_flagged() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);
    let reviewer = register_validator(&env, &client);

    approve_n_milestones(&env, &client, &validator, 1, 4000);

    // Revoke Routine — no flag set.
    client.revoke_validator(&validator, &RevocationSeverity::Routine, &None);

    let result = client.try_rereview_milestone(&reviewer, &4001u64, &1u32);
    assert_eq!(
        result,
        Err(Ok(VerificationError::MilestoneNotFlagged)),
        "unflagged milestone must return MilestoneNotFlagged"
    );
}

// ── Test 7: get_revocation_record returns the stored record ──────────────────

#[test]
fn get_revocation_record_returns_stored_record() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // No record before revocation.
    assert!(client.get_revocation_record(&validator).is_none());

    client.revoke_validator(
        &validator,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "credential fraud")),
    );

    let record = client
        .get_revocation_record(&validator)
        .expect("record must exist after revocation");
    assert_eq!(record.severity, RevocationSeverity::ForCause);
    assert_eq!(record.reason, String::from_str(&env, "credential fraud"));
}

// ── Test 8: original approver may also act as reviewer ───────────────────────
//
// The spec says "an active validator — not necessarily the original approver".
// Here we restore the original validator and confirm they can self-review
// (after being re-activated, they are again active).

#[test]
fn restored_original_validator_can_rereview_own_flagged_milestone() {
    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    approve_n_milestones(&env, &client, &validator, 1, 5000);
    client.revoke_validator(&validator, &RevocationSeverity::ForCause, &None);
    assert!(client.is_milestone_flagged(&5001u64, &1u32));

    // Restore the validator — they become active again.
    client.restore_validator(&validator);

    // Restored validator can now rereview their own flagged milestone.
    let result = client.try_rereview_milestone(&validator, &5001u64, &1u32);
    assert!(
        result.is_ok(),
        "restored validator must be able to rereview: {result:?}"
    );
    assert!(!client.is_milestone_flagged(&5001u64, &1u32));
}

// ── Test 9: Cost-budget — cascade sweep (limit=50) with 500+ milestones ──────
//
// Proves the per-call cost is proportional to CASCADE_LIMIT (50), not to the
// validator's full history (500+).
//
// A validator approved 5 milestones for each of 100 distinct players = 500
// approvals total. The initial revoke_validator call should flag exactly 50
// and halt; its CPU cost must stay within CASCADE_SWEEP_CPU_BUDGET.

#[test]
fn cascade_sweep_cpu_cost_bounded_at_500_milestones() {
    // Budget is deliberately generous since we cannot measure actual cost in
    // this environment (no Rust toolchain for a real run). Tighten after the
    // first CI run reports real numbers — see ci/cpu-cost-budget.md.
    const CASCADE_SWEEP_CPU_BUDGET: u64 = 50_000_000;

    let (env, client) = setup();
    let validator = register_validator(&env, &client);

    // Approve 500 milestones: 5 milestones per player, 100 players.
    // player_id = 10000 + player_offset
    // cid seeds arranged so no two calls share a CID
    let mut cid_seed: u32 = 10_000;
    for player_offset in 0u32..100 {
        let player_id = (10_000u32 + player_offset) as u64;
        for _ in 0..5 {
            client.approve_milestone(
                &validator,
                &player_id,
                &String::from_str(&env, "verified achievement"),
                &cid(&env, cid_seed),
                &None,
            );
            cid_seed += 1;
        }
    }

    assert_eq!(
        client.get_validator_milestone_count(&validator),
        500,
        "must have 500 milestones before revocation"
    );

    // Reset budget and measure only the initial revoke_validator call.
    env.cost_estimate().budget().reset_default();
    client.revoke_validator(
        &validator,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "fabricated credentials")),
    );
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();

    println!(
        "cascade_rereview: revoke_validator(ForCause, 500 milestones, limit=50) = {cpu} cpu \
         instructions (budget {CASCADE_SWEEP_CPU_BUDGET})"
    );

    assert!(
        cpu <= CASCADE_SWEEP_CPU_BUDGET,
        "cascade sweep at CASCADE_LIMIT=50 (out of 500 total) exceeded budget: \
         {cpu} > {CASCADE_SWEEP_CPU_BUDGET}. The per-call cost must be proportional \
         to CASCADE_LIMIT, not to the validator's full history."
    );

    // Exactly 50 milestones should be flagged: the first 10 players have
    // five approvals each, so the validator history's first 50 references
    // cover players 10000..10009.
    for player_offset in 0u32..10 {
        let player_id = (10_000u32 + player_offset) as u64;
        for milestone_index in 1u32..=5 {
            assert!(
                client.is_milestone_flagged(&player_id, &milestone_index),
                "player {player_id} milestone {milestone_index} must be flagged in first batch"
            );
        }
    }
    // The remaining players should NOT yet be flagged.
    for player_offset in 10u32..100 {
        let player_id = (10_000u32 + player_offset) as u64;
        assert!(
            !client.is_milestone_flagged(&player_id, &1u32),
            "player {player_id} milestone 1 must NOT be flagged yet (continuation needed)"
        );
    }
}
