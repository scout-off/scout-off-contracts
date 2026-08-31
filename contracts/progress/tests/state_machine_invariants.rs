//! Property-based invariant tests for the four-tier progress state machine
//! (issue #799).
//!
//! These tests prove that under any sequence of valid contract calls —
//! advance_level, reset_player_level, and combinations thereof — the player's
//! level:
//!   1. Only ever moves **forward by exactly one tier** (0→1, 1→2, 2→3), OR
//!   2. Is **explicitly reset** by an admin call to reset_player_level.
//!   3. Never skips a level (0→2, 1→3).
//!   4. Never silently reverses (2→1, 3→2) without an explicit reset.
//!
//! We use soroban testutils + hand-rolled sequence enumeration (equivalent to
//! proptest over a finite action domain) rather than proptest macros, which
//! don't work in no_std WASM context.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, Env};

// ── helpers ──────────────────────────────────────────────────────────────────

struct Harness {
    client: ProgressContractClient<'static>,
    /// Whitelisted caller for `advance_level`, registered as the *primary*
    /// VerificationContract (see `setup`).
    caller: Address,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);

    // Whitelist a test address on the *primary* (VerificationContract) path.
    //
    // The secondary (ScoutAccessContract) path cannot be used here: since #457
    // it cross-calls `get_milestone_count` on the configured verification
    // contract to validate `milestone_ref`, which requires a real deployed
    // contract and constrains which refs are accepted. The primary path skips
    // that check by design — the verification contract is the source of truth
    // for milestone data — so it is the right harness for level-transition
    // invariants, which are about the state machine, not milestone lookup.
    let caller = Address::generate(&env);
    client.set_verification_contract(&caller);

    Harness { client, caller }
}

/// Convert ProgressLevel to its numeric tier (0–3).
fn level_to_u32(l: &ProgressLevel) -> u32 {
    match l {
        ProgressLevel::Unverified => 0,
        ProgressLevel::VerifiedIdentity => 1,
        ProgressLevel::PerformanceMilestones => 2,
        ProgressLevel::EliteTier => 3,
    }
}

/// Assert the one-step-forward invariant between two consecutive levels unless
/// this was an explicit reset (milestone_ref == 0 in history entry).
/// For resets, enforce that new_level < old_level (a true rollback).
fn assert_valid_transition(old: &ProgressLevel, new: &ProgressLevel, is_reset: bool) {
    let old_n = level_to_u32(old);
    let new_n = level_to_u32(new);
    
    if is_reset {
        // Admin reset: new level must be strictly less than old level (true rollback)
        assert!(
            new_n < old_n,
            "reset violation: cannot reset from level {old_n} to level {new_n} (resets must move backward)"
        );
    } else {
        // Forward advance: must move exactly one tier forward
        assert_eq!(
            new_n,
            old_n + 1,
            "state machine violation: level jumped from {old_n} to {new_n} without a reset"
        );
    }
}

/// Read the full history and verify every consecutive pair satisfies the
/// one-step-forward-or-reset invariant.
fn assert_history_invariants(h: &Harness, player_id: u64) {
    let count = h.client.get_history_count(&player_id);
    if count < 2 {
        return;
    }
    for i in 1..count {
        let prev = h.client.get_history_entry(&player_id, &i);
        let next = h.client.get_history_entry(&player_id, &(i + 1));
        // milestone_ref == 0 marks an admin reset
        assert_valid_transition(&prev.new_level, &next.new_level, next.milestone_ref == 0);
    }
}

// ── core invariant tests ──────────────────────────────────────────────────────

/// Happy path: 0→1→2→3 each advance moves exactly one tier forward.
#[test]
fn test_sequential_forward_only() {
    let h = setup();
    let pid: u64 = 1;

    let l1 = h.client.advance_level(&h.caller, &pid, &1u32);
    assert_eq!(l1, ProgressLevel::VerifiedIdentity);

    let l2 = h.client.advance_level(&h.caller, &pid, &2u32);
    assert_eq!(l2, ProgressLevel::PerformanceMilestones);

    let l3 = h.client.advance_level(&h.caller, &pid, &3u32);
    assert_eq!(l3, ProgressLevel::EliteTier);

    assert_history_invariants(&h, pid);
}

/// Advancing past EliteTier returns AlreadyAtMaxLevel — level does not change.
#[test]
fn test_cannot_exceed_elite_tier() {
    use scoutchain_progress::ProgressError;
    let h = setup();
    let pid: u64 = 2;

    for i in 1..=3u32 {
        h.client.advance_level(&h.caller, &pid, &i);
    }
    assert_eq!(h.client.get_level(&pid), ProgressLevel::EliteTier);

    let result = h.client.try_advance_level(&h.caller, &pid, &4u32);
    assert_eq!(result, Err(Ok(ProgressError::AlreadyAtMaxLevel)));
    assert_eq!(
        h.client.get_level(&pid),
        ProgressLevel::EliteTier,
        "level must not change"
    );
    assert_history_invariants(&h, pid);
}

/// Admin reset mid-sequence: advance to 2, reset to 0, advance to 3.
/// History must show the reset entry and the subsequent forward steps.
#[test]
fn test_reset_mid_sequence_and_resume() {
    let h = setup();
    let pid: u64 = 3;

    h.client.advance_level(&h.caller, &pid, &1u32);
    h.client.advance_level(&h.caller, &pid, &2u32);
    assert_eq!(
        h.client.get_level(&pid),
        ProgressLevel::PerformanceMilestones
    );

    // Admin reset to Unverified
    h.client
        .reset_player_level(&pid, &ProgressLevel::Unverified);
    assert_eq!(h.client.get_level(&pid), ProgressLevel::Unverified);

    // Resume: must go through all three steps again
    h.client.advance_level(&h.caller, &pid, &1u32);
    h.client.advance_level(&h.caller, &pid, &2u32);
    h.client.advance_level(&h.caller, &pid, &3u32);
    assert_eq!(h.client.get_level(&pid), ProgressLevel::EliteTier);

    // Total 6 history entries: 2 advances + 1 reset + 3 more advances
    assert_eq!(h.client.get_history_count(&pid), 6);
    assert_history_invariants(&h, pid);
}

/// Reset to a mid-level (not Unverified) is valid and history is preserved.
#[test]
fn test_reset_to_mid_level() {
    let h = setup();
    let pid: u64 = 4;

    for i in 1..=3u32 {
        h.client.advance_level(&h.caller, &pid, &i);
    }
    assert_eq!(h.client.get_level(&pid), ProgressLevel::EliteTier);

    h.client
        .reset_player_level(&pid, &ProgressLevel::VerifiedIdentity);
    assert_eq!(h.client.get_level(&pid), ProgressLevel::VerifiedIdentity);

    // Can now re-advance from 1→2→3
    h.client.advance_level(&h.caller, &pid, &4u32);
    h.client.advance_level(&h.caller, &pid, &5u32);
    assert_eq!(h.client.get_level(&pid), ProgressLevel::EliteTier);

    assert_history_invariants(&h, pid);
}

/// Multiple players are fully independent — one player's state never affects another's.
#[test]
fn test_multiple_players_independent() {
    let h = setup();

    // Player A: full progression
    for i in 1..=3u32 {
        h.client.advance_level(&h.caller, &1u64, &i);
    }
    // Player B: only one step
    h.client.advance_level(&h.caller, &2u64, &1u32);
    // Player C: reset immediately (starts at 0, stays at 0)
    h.client
        .reset_player_level(&3u64, &ProgressLevel::Unverified);

    assert_eq!(h.client.get_level(&1u64), ProgressLevel::EliteTier);
    assert_eq!(h.client.get_level(&2u64), ProgressLevel::VerifiedIdentity);
    assert_eq!(h.client.get_level(&3u64), ProgressLevel::Unverified);

    assert_history_invariants(&h, 1);
    assert_history_invariants(&h, 2);
}

/// Exhaustive sequence enumeration: all permutations of up to 4 actions from
/// {Advance, Reset(Unverified), Reset(VerifiedIdentity)} on a single player.
///
/// After every step we assert:
///   - actual level == tracked expected level
///   - full history satisfies the one-step-forward-or-reset invariant
///
/// This is the property-based core of the test suite.
#[test]
fn test_exhaustive_action_sequences() {
    #[derive(Clone, Copy, Debug)]
    enum Action {
        Advance,
        ResetUnverified,
        ResetVerifiedIdentity,
    }

    let actions = [
        Action::Advance,
        Action::ResetUnverified,
        Action::ResetVerifiedIdentity,
    ];

    // Generate all sequences of length 1..=4
    let mut sequences: std::vec::Vec<std::vec::Vec<Action>> = std::vec::Vec::new();
    for a0 in &actions {
        sequences.push(std::vec![*a0]);
        for a1 in &actions {
            sequences.push(std::vec![*a0, *a1]);
            for a2 in &actions {
                sequences.push(std::vec![*a0, *a1, *a2]);
                for a3 in &actions {
                    sequences.push(std::vec![*a0, *a1, *a2, *a3]);
                }
            }
        }
    }

    let pid: u64 = 99;
    let mut milestone_counter: u32 = 0;

    for seq in &sequences {
        let h = setup();
        let mut expected = ProgressLevel::Unverified;

        for action in seq {
            match action {
                Action::Advance => {
                    // Skip if already at max
                    if expected == ProgressLevel::EliteTier {
                        let result = h.client.try_advance_level(&h.caller, &pid, &0u32);
                        assert!(result.is_err(), "advance at EliteTier must fail");
                        // level unchanged
                    } else {
                        milestone_counter += 1;
                        h.client.advance_level(&h.caller, &pid, &milestone_counter);
                        expected = expected.next().unwrap();
                        let actual = h.client.get_level(&pid);
                        assert_eq!(
                            actual, expected,
                            "after Advance: expected {expected:?} got {actual:?} in seq {seq:?}"
                        );
                    }
                }
                Action::ResetUnverified => {
                    h.client
                        .reset_player_level(&pid, &ProgressLevel::Unverified);
                    expected = ProgressLevel::Unverified;
                    assert_eq!(h.client.get_level(&pid), expected);
                }
                Action::ResetVerifiedIdentity => {
                    h.client
                        .reset_player_level(&pid, &ProgressLevel::VerifiedIdentity);
                    expected = ProgressLevel::VerifiedIdentity;
                    assert_eq!(h.client.get_level(&pid), expected);
                }
            }
            assert_history_invariants(&h, pid);
        }
    }
}

/// Pausing the contract blocks advance_level and reset_player_level.
#[test]
fn test_paused_contract_blocks_all_mutations() {
    use scoutchain_progress::ProgressError;
    let h = setup();
    let pid: u64 = 50;

    h.client.advance_level(&h.caller, &pid, &1u32);
    h.client.pause_contract();

    let r1 = h.client.try_advance_level(&h.caller, &pid, &2u32);
    assert_eq!(r1, Err(Ok(ProgressError::ContractPaused)));

    let r2 = h
        .client
        .try_reset_player_level(&pid, &ProgressLevel::Unverified);
    assert_eq!(r2, Err(Ok(ProgressError::ContractPaused)));

    // Level unchanged
    assert_eq!(h.client.get_level(&pid), ProgressLevel::VerifiedIdentity);
}
