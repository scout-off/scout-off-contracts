//! Merkle commitment tests for player progress history (issue #700).
//!
//! `get_progress_root` / `get_history_proof` / `verify_history_proof` let any
//! caller independently check that a `ProgressEntry` is genuinely part of a
//! player's on-chain history, without trusting the RPC node that served the
//! query. These tests prove the commitment is sound (genuine proofs verify)
//! and safe under adversarial input (forged, stale, or malformed proofs are
//! rejected — never accepted, never panicking).
//!
//! Note: this file intentionally never names `ProgressEntry`, `ProgressError`,
//! or `HistoryProofStep` as import paths — every value of those types here
//! comes from a generated client call and is used via type inference or
//! field access, matching the pattern already used by the other integration
//! test files in this directory (`cursor_pagination_tests.rs`,
//! `state_machine_invariants.rs`).

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Vec};

struct Harness {
    env: Env,
    client: ProgressContractClient<'static>,
    /// Whitelisted caller registered on the *primary* (VerificationContract)
    /// path, so `advance_level` works without deploying a real verification
    /// contract. The secondary path cross-calls `get_milestone_count` (#457)
    /// and therefore needs a real contract; these tests only care about the
    /// history commitment, not milestone validation.
    caller: Address,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);
    let caller = Address::generate(&env);
    client.set_verification_contract(&caller);
    Harness {
        env,
        client,
        caller,
    }
}

// ── genuine proofs verify ───────────────────────────────────────────────────

#[test]
fn test_genuine_proof_verifies_true_for_every_entry() {
    let h = setup();
    let player_id = 1u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);
    h.client.advance_level(&h.caller, &player_id, &2u32);
    h.client.advance_level(&h.caller, &player_id, &3u32);

    for index in 1..=3u32 {
        let entry = h.client.get_history_entry(&player_id, &index);
        let proof = h.client.get_history_proof(&player_id, &index);
        assert!(
            h.client.verify_history_proof(&player_id, &entry, &proof),
            "entry {index} must verify against the current root"
        );
    }
}

// ── forged entry rejected ───────────────────────────────────────────────────

#[test]
fn test_single_field_flip_fails_verification() {
    let h = setup();
    let player_id = 2u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);
    h.client.advance_level(&h.caller, &player_id, &2u32);

    let mut entry = h.client.get_history_entry(&player_id, &1u32);
    let proof = h.client.get_history_proof(&player_id, &1u32);

    // Sanity: the genuine, unmodified entry verifies before we touch it.
    assert!(h.client.verify_history_proof(&player_id, &entry, &proof));

    // Flip one field (new_level) and confirm the proof no longer verifies —
    // the leaf hash covers every field, so a one-field forgery is enough.
    entry.new_level = ProgressLevel::EliteTier;
    assert!(
        !h.client.verify_history_proof(&player_id, &entry, &proof),
        "a forged entry (single field changed) must not verify"
    );
}

#[test]
fn test_flipped_milestone_ref_fails_verification() {
    let h = setup();
    let player_id = 21u64;
    h.client.advance_level(&h.caller, &player_id, &7u32);

    let mut entry = h.client.get_history_entry(&player_id, &1u32);
    let proof = h.client.get_history_proof(&player_id, &1u32);
    assert!(h.client.verify_history_proof(&player_id, &entry, &proof));

    entry.milestone_ref = entry.milestone_ref.wrapping_add(1);
    assert!(!h.client.verify_history_proof(&player_id, &entry, &proof));
}

// ── stale root rejected ─────────────────────────────────────────────────────

#[test]
fn test_proof_against_stale_root_rejected_after_new_append() {
    let h = setup();
    let player_id = 3u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);

    let entry = h.client.get_history_entry(&player_id, &1u32);
    let stale_proof = h.client.get_history_proof(&player_id, &1u32);
    assert!(h
        .client
        .verify_history_proof(&player_id, &entry, &stale_proof));

    // Appending a second entry changes the root (the tree shape itself
    // changes, n=1 -> n=2), even though entry 1's own data is untouched.
    h.client.advance_level(&h.caller, &player_id, &2u32);

    assert!(
        !h.client
            .verify_history_proof(&player_id, &entry, &stale_proof),
        "a proof computed against the pre-append root must not verify against \
         the post-append root"
    );

    // A freshly generated proof against the new root verifies correctly.
    let fresh_proof = h.client.get_history_proof(&player_id, &1u32);
    assert!(h
        .client
        .verify_history_proof(&player_id, &entry, &fresh_proof));
}

// ── malformed proofs rejected without panicking ─────────────────────────────

#[test]
fn test_empty_proof_rejected_without_panicking() {
    let h = setup();
    let player_id = 4u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);
    h.client.advance_level(&h.caller, &player_id, &2u32);

    let entry = h.client.get_history_entry(&player_id, &1u32);
    let empty = Vec::new(&h.env);
    assert!(
        !h.client.verify_history_proof(&player_id, &entry, &empty),
        "an empty proof against a multi-entry history must not verify"
    );
}

#[test]
fn test_wrong_length_proof_rejected_without_panicking() {
    let h = setup();
    let player_id = 5u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);
    h.client.advance_level(&h.caller, &player_id, &2u32);
    h.client.advance_level(&h.caller, &player_id, &3u32);
    let entry = h.client.get_history_entry(&player_id, &1u32); // real proof depth 2

    // A proof generated for a different (shallower) player's single-entry
    // history has depth 0 — structurally valid but the wrong length here.
    let shallow_player = 6u64;
    h.client.advance_level(&h.caller, &shallow_player, &1u32);
    let too_short = h.client.get_history_proof(&shallow_player, &1u32);
    assert!(!h
        .client
        .verify_history_proof(&player_id, &entry, &too_short));

    // An oversized proof, built by repeatedly appending a valid proof to
    // itself, must also be rejected — proving an adversarial caller cannot
    // force unbounded verification cost by submitting an arbitrarily long
    // proof Vec (the contract enforces a fixed step cap).
    let mut too_long = h.client.get_history_proof(&player_id, &1u32);
    for _ in 0..40 {
        let extra = h.client.get_history_proof(&player_id, &1u32);
        too_long.append(&extra);
    }
    assert!(too_long.len() > 32, "test setup should exceed the step cap");
    assert!(!h.client.verify_history_proof(&player_id, &entry, &too_long));
}

// ── get_progress_root defaults / verify_history_proof error case ───────────

#[test]
fn test_get_progress_root_defaults_to_zero_for_unknown_player() {
    let h = setup();
    let root = h.client.get_progress_root(&999u64);
    assert_eq!(root, BytesN::from_array(&h.env, &[0u8; 32]));
}

#[test]
fn test_verify_history_proof_errors_for_player_with_no_history() {
    let h = setup();
    // Borrow a structurally valid (entry, proof) pair from a different
    // player — verify_history_proof must reject before even inspecting
    // them, because the target player has no root to check against at all.
    let other_player = 1u64;
    h.client.advance_level(&h.caller, &other_player, &1u32);
    let entry = h.client.get_history_entry(&other_player, &1u32);
    let proof = h.client.get_history_proof(&other_player, &1u32);

    let never_used_player = 999u64;
    let result = h
        .client
        .try_verify_history_proof(&never_used_player, &entry, &proof);
    assert!(
        result.is_err(),
        "a player with no committed root at all must error, distinct from \
         an existing player's proof simply failing to verify"
    );
}

#[test]
fn test_get_history_proof_errors_for_out_of_range_index() {
    let h = setup();
    let player_id = 7u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);

    assert!(h.client.try_get_history_proof(&player_id, &0u32).is_err());
    assert!(h.client.try_get_history_proof(&player_id, &2u32).is_err());
    assert!(h.client.try_get_history_proof(&999u64, &1u32).is_err());
}

// ── beyond the 3-entry tier cap (admin resets keep appending) ──────────────

#[test]
fn test_history_proof_correct_beyond_three_entries_via_resets() {
    let h = setup();
    let player_id = 42u64;
    h.client.advance_level(&h.caller, &player_id, &1u32);
    h.client.advance_level(&h.caller, &player_id, &2u32);
    h.client.advance_level(&h.caller, &player_id, &3u32);
    // Dispute-resolution resets keep appending to history beyond the
    // three-tier cap — the commitment scheme must not assume n <= 3.
    h.client
        .reset_player_level(&player_id, &ProgressLevel::Unverified);
    h.client
        .reset_player_level(&player_id, &ProgressLevel::PerformanceMilestones);

    assert_eq!(h.client.get_history_count(&player_id), 5);
    for index in 1..=5u32 {
        let entry = h.client.get_history_entry(&player_id, &index);
        let proof = h.client.get_history_proof(&player_id, &index);
        assert!(
            h.client.verify_history_proof(&player_id, &entry, &proof),
            "entry {index} of 5 must verify against the final root"
        );
    }
}

// ── property/fuzz-style sweep across many synthetic players ────────────────

/// Deterministic LCG (xorshift-ish multiply-add) — `Env`-forbidden real
/// randomness sources aren't available in a Soroban test context, so this
/// gives a reproducible, non-hand-picked sequence of history lengths instead.
struct Lcg(u64);
impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 33) as u32
    }
}

#[test]
fn test_property_every_appended_entry_verifies_against_final_root() {
    let h = setup();
    let mut rng = Lcg(0x9E3779B97F4A7C15);

    for offset in 0u64..50 {
        let player_id = 10_000 + offset;
        // 1..=3 entries, respecting the real four-tier model's cap on
        // ordinary advance_level sequences (Unverified -> ... -> EliteTier).
        let n = 1 + (rng.next_u32() % 3);
        for i in 1..=n {
            h.client.advance_level(&h.caller, &player_id, &i);
        }

        for index in 1..=n {
            let entry = h.client.get_history_entry(&player_id, &index);
            let proof = h.client.get_history_proof(&player_id, &index);
            assert!(
                h.client.verify_history_proof(&player_id, &entry, &proof),
                "player {player_id} entry {index} of {n} failed to verify \
                 against its final root"
            );
        }
    }
}
