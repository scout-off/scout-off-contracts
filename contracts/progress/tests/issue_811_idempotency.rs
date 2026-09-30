//! Retry-after-uncertain-failure audit for `progress.advance_level`
//! (issue #811 follow-up).
//!
//! # Why this file exists
//!
//! `progress.advance_level` is the shared cross-contract call target that both
//! `verification.approve_milestone` and `scout_access.confirm_trial_offer`
//! invoke, and both wrap it in `try_advance_level` so that a failure surfaces
//! as `ProgressCallFailed` (verification code 12, scout_access code 14) rather
//! than an unconditional trap. `docs/CONTRACT_REFERENCE.md`'s "Error Handling —
//! ProgressCallFailed" section tells an operator who sees that error to re-run
//! the wiring script and **retry the originating call**.
//!
//! That recovery guidance is only safe if a retry cannot double-apply. The
//! dangerous case is not a clean failure — it is an *uncertain* one: a
//! submitter loses its RPC connection, or times out, after the transaction was
//! broadcast but before it observed the result. It cannot tell whether the
//! first `advance_level` committed. `scout_access.confirm_trial_offer` protects
//! its own retry path with `DataKey::ConfirmationNonce(String)`; `progress` has
//! no equivalent nonce key.
//!
//! # Determination
//!
//! `advance_level` is **NOT internally idempotent, and deliberately so.** It is
//! a monotonic *state-machine step*, not a keyed mutation: it reads the current
//! level, computes `current.next()`, and appends a history entry. Calling it
//! twice with the *same* `milestone_ref` advances two tiers (0 -> 1 -> 2) and
//! appends two history entries, because nothing in the function keys off
//! `milestone_ref` — there is no per-`milestone_ref` dedup record.
//!
//! This does not make the system unsafe, because a *blind* replay is not
//! reachable through either production caller. The protection lives at the
//! caller, one layer up, and it is a genuine safety property rather than an
//! accident:
//!
//!   * `verification.approve_milestone` marks `DataKey::EvidenceUsed(hash)`
//!     before it calls `advance_level`, and rejects an already-recorded hash
//!     with `DuplicateEvidence`. A retry of the *same* milestone approval
//!     therefore never reaches `advance_level` a second time. Crucially, if the
//!     first attempt failed at the `advance_level` step, the host-level revert
//!     rolls `EvidenceUsed` back too, so the retry is free to proceed — exactly
//!     once.
//!   * `scout_access.confirm_trial_offer` checks
//!     `DataKey::ConfirmationNonce(offer_key)` before calling `advance_level`
//!     and persists it after, so a retry short-circuits.
//!
//! So the "atomic all-or-nothing" property that makes retry safe is supplied by
//! the caller's dedup key plus Soroban's whole-transaction revert, and
//! `advance_level` itself contributes the *milestone_ref existence* check on
//! the secondary path. The residual risk is a **direct** call by a
//! whitelisted contract that does not implement its own dedup — which is why
//! the tests below pin the double-apply behaviour as an explicit, documented
//! contract rather than leaving it as an untested assumption.
//!
//! Per the issue's own acceptance note, a repeat call that safely yields
//! `InvalidProgressTransition` / `AlreadyAtMaxLevel` counts as safe. That is
//! precisely what happens once a player reaches the top tier
//! (`test_replay_at_max_level_is_inert`), and on the secondary path when the
//! `milestone_ref` cannot be justified
//! (`test_secondary_caller_replay_with_unbacked_milestone_ref_is_rejected`).
//!
//! Companion determination for `registration.set_player_level` lives in
//! `contracts/registration/tests/issue_811_idempotency.rs`.
//!
//! Run: `cargo test -p scoutchain-progress --test issue_811_idempotency`

use scoutchain_progress::{
    DataKey, ProgressContract, ProgressContractClient, ProgressEntry, ProgressError,
};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

// ── harness ──────────────────────────────────────────────────────────────────

struct Harness {
    env: Env,
    client: ProgressContractClient<'static>,
    /// Address whitelisted as the *primary* `advance_level` caller (stands in
    /// for the verification contract). The primary path does not cross-call
    /// back into verification, so a plain generated address is sufficient.
    caller: Address,
    /// The progress contract's own address, for raw storage inspection.
    contract_id: Address,
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
        contract_id: id,
    }
}

/// Read `HistoryCounter` straight out of persistent storage, bypassing the
/// getter, so the assertion cannot be satisfied by a getter-side default.
fn raw_history_counter(h: &Harness, player_id: u64) -> u32 {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .get(&DataKey::HistoryCounter(player_id))
            .unwrap_or(0u32)
    })
}

/// Read the sharded `HistoryPage` for a single page straight out of storage.
fn raw_history_page(h: &Harness, player_id: u64, page: u32) -> Vec<ProgressEntry> {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .get(&DataKey::HistoryPage(player_id, page))
            .unwrap_or_else(|| Vec::new(&h.env))
    })
}

/// Read `PlayerLevel` straight out of persistent storage.
fn raw_history_vec(h: &Harness, player_id: u64) -> Vec<ProgressEntry> {
    let counter = raw_history_counter(h, player_id);
    let page_size = 8u32;
    let mut reconstructed = Vec::new(&h.env);
    if counter == 0 {
        return reconstructed;
    }
    for page in 0..=((counter - 1) / page_size) {
        let entries = raw_history_page(h, player_id, page);
        for i in 0..entries.len() {
            reconstructed.push_back(entries.get(i).unwrap());
        }
    }
    reconstructed
}

fn raw_player_level(h: &Harness, player_id: u64) -> Option<ProgressLevel> {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .get(&DataKey::PlayerLevel(player_id))
    })
}

/// The canonical history is sharded into bounded pages, but every entry still
/// follows the same logical order as `HistoryEntry(player_id, i)` plus the
/// counter. A divergence would mean a partially-applied append.
fn assert_history_representations_agree(h: &Harness, player_id: u64) {
    let counter = raw_history_counter(h, player_id);
    let page_size = 8u32;
    let mut reconstructed = Vec::new(&h.env);
    for page in 0..=((counter.saturating_sub(1)) / page_size) {
        let entries = raw_history_page(h, player_id, page);
        for i in 0..entries.len() {
            reconstructed.push_back(entries.get(i).unwrap());
        }
    }
    assert_eq!(
        counter,
        reconstructed.len(),
        "HistoryCounter and reconstructed page history disagree on entry count"
    );
    for i in 1..=counter {
        let indexed = h.client.get_history_entry(&player_id, &i);
        let from_page = reconstructed.get(i - 1).expect("page entry must exist");
        assert_eq!(
            indexed.new_level, from_page.new_level,
            "indexed and paged history disagree at index {i}"
        );
        assert_eq!(
            indexed.milestone_ref, from_page.milestone_ref,
            "indexed and paged history disagree at index {i}"
        );
    }
}

#[test]
fn test_history_is_sharded_into_fixed_pages() {
    let h = setup();
    let pid: u64 = 99;
    // A player can advance at most three tiers. Reset after each complete
    // progression so the test creates 20 valid history entries while still
    // exercising multiple fixed-size pages.
    for milestone in 1..=15u32 {
        h.client.advance_level(&h.caller, &pid, &milestone);
        if milestone % 3 == 0 {
            h.client
                .reset_player_level(&pid, &ProgressLevel::Unverified);
        }
    }

    let total = raw_history_counter(&h, pid);
    assert_eq!(total, 20);
    assert_eq!(raw_history_page(&h, pid, 0).len(), 8);
    assert_eq!(raw_history_page(&h, pid, 1).len(), 8);
    assert_eq!(raw_history_page(&h, pid, 2).len(), 4);
}

// ── tests ────────────────────────────────────────────────────────────────────

/// **The core finding.** A blind replay of `advance_level` with an identical
/// `milestone_ref` is NOT a no-op: it advances a second tier and appends a
/// second history entry. This test pins that behaviour so that any future
/// change which silently makes the call idempotent (or, worse, makes it
/// partially idempotent) has to update this file deliberately.
#[test]
fn test_duplicate_advance_level_with_same_milestone_ref_double_applies() {
    let h = setup();
    let pid: u64 = 1;
    let milestone_ref: u32 = 1;

    let first = h.client.advance_level(&h.caller, &pid, &milestone_ref);
    assert_eq!(first, ProgressLevel::VerifiedIdentity);
    assert_eq!(raw_history_counter(&h, pid), 1);

    // Replay the *exact same* call, as an operator following the
    // ProgressCallFailed recovery steps after an uncertain failure would.
    let second = h.client.advance_level(&h.caller, &pid, &milestone_ref);

    assert_eq!(
        second,
        ProgressLevel::PerformanceMilestones,
        "replay advanced a second tier — advance_level is a state-machine step, \
         not a keyed idempotent mutation"
    );
    assert_eq!(
        raw_history_counter(&h, pid),
        2,
        "replay appended a second history entry"
    );

    // Both entries carry the same milestone_ref: nothing in the stored history
    // distinguishes the replay from a legitimate second advance. This is the
    // precise reason caller-side dedup is mandatory.
    let packed = raw_history_vec(&h, pid);
    assert_eq!(packed.len(), 2);
    assert_eq!(packed.get(0).unwrap().milestone_ref, milestone_ref);
    assert_eq!(packed.get(1).unwrap().milestone_ref, milestone_ref);
    assert_eq!(
        packed.get(1).unwrap().old_level,
        ProgressLevel::VerifiedIdentity,
        "the second entry chains off the first — history stays internally consistent"
    );

    assert_eq!(
        raw_player_level(&h, pid),
        Some(ProgressLevel::PerformanceMilestones)
    );
    assert_history_representations_agree(&h, pid);
}

/// Once a player is at the top tier, further replays are inert: they return
/// `AlreadyAtMaxLevel` and write nothing. Per the issue's acceptance note this
/// is the "safe" outcome, and it bounds the blast radius of any replay loop to
/// at most three extra tiers.
#[test]
fn test_replay_at_max_level_is_inert() {
    let h = setup();
    let pid: u64 = 2;

    for i in 1..=3u32 {
        h.client.advance_level(&h.caller, &pid, &i);
    }
    assert_eq!(h.client.get_level(&pid), ProgressLevel::EliteTier);

    let counter_before = raw_history_counter(&h, pid);
    let level_before = raw_player_level(&h, pid);

    // Hammer the endpoint the way a retry loop would.
    for _ in 0..5 {
        let result = h.client.try_advance_level(&h.caller, &pid, &3u32);
        assert_eq!(
            result,
            Err(Ok(ProgressError::AlreadyAtMaxLevel)),
            "replay past the top tier must fail closed"
        );
    }

    assert_eq!(
        raw_history_counter(&h, pid),
        counter_before,
        "a rejected replay must not append history"
    );
    assert_eq!(
        raw_player_level(&h, pid),
        level_before,
        "a rejected replay must not change the stored level"
    );
    assert_history_representations_agree(&h, pid);
}

/// A rejected `advance_level` must leave **no** partial state behind — no
/// history entry, no counter bump, no level write, no packed-vec append. This
/// mirrors the "no partial state" assertions in
/// `contracts/verification/tests/issue_811_idempotency.rs`, reading raw
/// `DataKey`s rather than trusting the getters.
#[test]
fn test_rejected_advance_leaves_no_partial_state() {
    let h = setup();
    let pid: u64 = 3;

    // Untouched player: nothing should exist yet.
    assert_eq!(raw_history_counter(&h, pid), 0);
    assert!(raw_player_level(&h, pid).is_none());

    h.client.pause_contract();

    let result = h.client.try_advance_level(&h.caller, &pid, &1u32);
    assert_eq!(result, Err(Ok(ProgressError::ContractPaused)));

    assert_eq!(
        raw_history_counter(&h, pid),
        0,
        "HistoryCounter must not be written by a rejected call"
    );
    assert!(
        raw_history_vec(&h, pid).is_empty(),
        "HistoryVec must not be written by a rejected call"
    );
    assert!(
        raw_player_level(&h, pid).is_none(),
        "PlayerLevel must not be written by a rejected call"
    );
    assert!(
        !h.env.as_contract(&h.contract_id, || {
            h.env
                .storage()
                .persistent()
                .has(&DataKey::HistoryEntry(pid, 1))
        }),
        "no HistoryEntry(pid, 1) should exist after a rejected call"
    );
}

/// Who can drive a replay at all? Two properties bound the exposure, and the
/// second one is a sharp edge worth recording explicitly.
///
/// 1. With **no** peer wired, `advance_level` fails closed with
///    `NotInitialized` — there is no open fallback caller.
/// 2. `advance_level` requires auth from the *stored* whitelist address, not
///    from the `caller` argument. The `caller` parameter is only recorded as
///    `updated_by` in the history entry. So under a real (non-mocked) auth
///    context, a stranger cannot produce the signature the contract demands,
///    and the call aborts — but note that the address written into the audit
///    trail is attacker-chosen if the whitelisted contract ever forwards an
///    unvalidated argument. Callers must therefore pass a trustworthy
///    `caller`; the audit trail's `updated_by` is not self-authenticating.
#[test]
fn test_replay_requires_a_wired_and_authorized_caller() {
    // (1) Nothing wired at all.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);

    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_advance_level(&stranger, &1u64, &1u32),
        Err(Ok(ProgressError::NotInitialized)),
        "with no peer configured advance_level must fail closed"
    );
    assert!(
        env.as_contract(&id, || {
            env.storage()
                .persistent()
                .get::<DataKey, ProgressLevel>(&DataKey::PlayerLevel(1u64))
        })
        .is_none(),
        "a call rejected for missing wiring must not write a level"
    );

    // (2) Wired, but auth is genuinely enforced rather than mocked.
    let h = setup();
    let pid: u64 = 4;
    h.client.advance_level(&h.caller, &pid, &1u32);
    let counter_before = raw_history_counter(&h, pid);

    h.env.set_auths(&[]);
    let outsider = Address::generate(&h.env);
    assert!(
        h.client.try_advance_level(&outsider, &pid, &1u32).is_err(),
        "without the whitelisted contract's signature the replay must fail"
    );

    assert_eq!(
        raw_history_counter(&h, pid),
        counter_before,
        "an unauthorised replay must not append history"
    );
    assert_eq!(
        raw_player_level(&h, pid),
        Some(ProgressLevel::VerifiedIdentity),
        "an unauthorised replay must not change the level"
    );
}

/// On the **secondary** (scout_access) path, `advance_level` validates that the
/// supplied `milestone_ref` is backed by a real milestone in the verification
/// contract (#457). A replay carrying a fabricated or not-yet-existing
/// `milestone_ref` is rejected with `InvalidProgressTransition` — the issue's
/// stated "safe" outcome — and writes nothing.
///
/// This guard is a genuine cross-contract read, so the test registers a **real**
/// verification contract rather than a bare address: the `get_milestone_count`
/// call must actually resolve for the guard to return a contract error instead
/// of trapping.
#[test]
fn test_secondary_caller_replay_with_unbacked_milestone_ref_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let progress_id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &progress_id);
    client.initialize(&admin);

    // A real verification contract, so the #457 milestone-existence cross-call
    // resolves. No milestones are approved, so its count stays 0.
    let verification_id = env.register(VerificationContract, ());
    VerificationContractClient::new(&env, &verification_id).initialize(&admin);
    client.set_verification_contract(&verification_id);

    let scout_access = Address::generate(&env);
    client.set_scout_access_contract(&scout_access);

    let pid: u64 = 5;

    // milestone_ref == 0 is rejected outright, and any ref beyond the real
    // milestone count is rejected too — a replay cannot invent justification.
    for bad_ref in [0u32, 1u32, 99u32] {
        assert_eq!(
            client.try_advance_level(&scout_access, &pid, &bad_ref),
            Err(Ok(ProgressError::InvalidProgressTransition)),
            "secondary path must reject unbacked milestone_ref {bad_ref}"
        );
    }

    let counter: u32 = env.as_contract(&progress_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::HistoryCounter(pid))
            .unwrap_or(0u32)
    });
    assert_eq!(
        counter, 0,
        "a rejected secondary-path call must not append history"
    );
    assert!(
        env.as_contract(&progress_id, || {
            env.storage()
                .persistent()
                .get::<DataKey, ProgressLevel>(&DataKey::PlayerLevel(pid))
        })
        .is_none(),
        "a rejected secondary-path call must not write a level"
    );
}

/// Interleaving an admin reset with replays keeps the two history
/// representations consistent and keeps the level exactly where the last
/// successful mutation put it. `reset_player_level` records its entry with
/// `milestone_ref == 0`, so a reset is always distinguishable from an advance
/// in the audit trail — which is what makes post-incident reconciliation of a
/// suspected double-apply possible.
#[test]
fn test_reset_then_replay_keeps_history_consistent() {
    let h = setup();
    let pid: u64 = 6;

    h.client.advance_level(&h.caller, &pid, &1u32);
    h.client.advance_level(&h.caller, &pid, &2u32);
    assert_eq!(
        h.client.get_level(&pid),
        ProgressLevel::PerformanceMilestones
    );

    h.client
        .reset_player_level(&pid, &ProgressLevel::Unverified);
    assert_eq!(raw_player_level(&h, pid), Some(ProgressLevel::Unverified));

    let after_reset = raw_history_counter(&h, pid);
    assert_eq!(after_reset, 3, "the reset itself is recorded in history");
    let packed = raw_history_vec(&h, pid);
    assert_eq!(
        packed.get(2).unwrap().milestone_ref,
        0,
        "a reset entry is tagged with milestone_ref == 0"
    );

    // Replaying the original advance after a reset re-applies it from the reset
    // level — again showing the call is a step, not a keyed mutation.
    let level = h.client.advance_level(&h.caller, &pid, &1u32);
    assert_eq!(level, ProgressLevel::VerifiedIdentity);
    assert_eq!(raw_history_counter(&h, pid), 4);

    assert_history_representations_agree(&h, pid);
}

/// Whatever the replay pattern, the two history representations must never
/// diverge. A divergence would mean an append was only half-applied, which is
/// the failure mode that would make a retry genuinely dangerous rather than
/// merely non-idempotent.
#[test]
fn test_history_representations_never_diverge_under_replay() {
    let h = setup();
    let pid: u64 = 7;

    // Replay the same milestone_ref until the state machine saturates.
    for _ in 0..3 {
        h.client.advance_level(&h.caller, &pid, &1u32);
        assert_history_representations_agree(&h, pid);
    }
    assert_eq!(h.client.get_level(&pid), ProgressLevel::EliteTier);

    // Further replays are rejected and must not disturb the invariant.
    for _ in 0..3 {
        let _ = h.client.try_advance_level(&h.caller, &pid, &1u32);
        assert_history_representations_agree(&h, pid);
    }

    assert_eq!(
        raw_history_counter(&h, pid),
        3,
        "exactly three successful advances were possible"
    );
}
