//! Contract-upgrade rehearsal harness — `progress` contract.
//!
//! See `contracts/registration/tests/upgrade_rehearsal.rs` for the full write-up
//! of what this family of harnesses is, why it exists (turning the prose
//! "What survives an upgrade" table in `docs/DEPLOYMENT.md` into automated
//! assertions), and the WASM-swap mechanism and its limitation (a genuinely
//! different v2 artifact cannot be built in this toolchain-less sandbox, so the
//! real `upgrade()` code path is driven with an empty-bytes WASM blob).
//!
//! Run: `cargo test -p scoutchain-progress --test upgrade_rehearsal`.
//!
//! `progress` stores each player's level in **persistent** storage (survives the
//! swap) and holds three cross-contract links in **instance** storage
//! (`VerificationContract`, `RegistrationContract`, `ScoutAccessContract`) that
//! `docs/DEPLOYMENT.md` says to re-wire after an upgrade. The happy-path test
//! re-wires all three; the deliberately-broken test proves the harness catches
//! an operator who forgets to re-verify the instance `Paused` flag.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, Env,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    env: Env,
    progress: ProgressContractClient<'static>,
    /// Dummy address whitelisted as the primary `advance_level` caller.
    verifier: Address,
}

fn rehearse_upgrade(h: &Harness) {
    let new_wasm_hash = h.env.deployer().upload_contract_wasm(Bytes::new(&h.env));
    h.progress.upgrade(&new_wasm_hash);
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &id);
    progress.initialize(&admin);

    // Whitelist a primary caller so we can advance player levels. advance_level's
    // primary-caller path does not cross-call the verification contract, so a
    // plain generated address is sufficient here.
    let verifier = Address::generate(&env);
    progress.set_verification_contract(&verifier);

    Harness {
        env,
        progress,
        verifier,
    }
}

/// Seed three players at three different levels so the survival assertions cover
/// more than a single value.
fn seed(h: &Harness) {
    // Player 1 -> VerifiedIdentity (1 advance).
    h.progress.advance_level(&h.verifier, &1u64, &1u32);
    // Player 2 -> PerformanceMilestones (2 advances).
    h.progress.advance_level(&h.verifier, &2u64, &1u32);
    h.progress.advance_level(&h.verifier, &2u64, &2u32);
    // Player 3 -> EliteTier (3 advances).
    h.progress.advance_level(&h.verifier, &3u64, &1u32);
    h.progress.advance_level(&h.verifier, &3u64, &2u32);
    h.progress.advance_level(&h.verifier, &3u64, &3u32);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full rehearsal for `progress`.
#[test]
fn test_progress_upgrade_preserves_state() {
    let h = setup();
    seed(&h);

    // --- Snapshot (pre-upgrade) ---
    let l1 = h.progress.get_level(&1u64);
    let l2 = h.progress.get_level(&2u64);
    let l3 = h.progress.get_level(&3u64);
    let hist1 = h.progress.get_history_count(&1u64);
    let hist3 = h.progress.get_history_count(&3u64);
    let health = h.progress.health();

    assert_eq!(l1, ProgressLevel::VerifiedIdentity);
    assert_eq!(l2, ProgressLevel::PerformanceMilestones);
    assert_eq!(l3, ProgressLevel::EliteTier);
    assert_eq!(hist1, 1);
    assert_eq!(hist3, 3);
    assert!(health.initialized && !health.paused);

    // --- Upgrade ---
    rehearse_upgrade(&h);

    // Re-wire the verification link (the one `advance_level` requires). The
    // registration / scout_access links are re-wired further down, after the
    // behavioural check, so `advance_level` does not try to sync a level change
    // into a dummy registration address.
    h.progress.set_verification_contract(&h.verifier);

    // --- Assert: persistent storage survived (player levels + history) ---
    assert_eq!(h.progress.get_level(&1u64), l1);
    assert_eq!(h.progress.get_level(&2u64), l2);
    assert_eq!(h.progress.get_level(&3u64), l3);
    assert_eq!(h.progress.get_history_count(&1u64), hist1);
    assert_eq!(h.progress.get_history_count(&3u64), hist3);

    // --- Assert: instance flags survived ---
    assert_eq!(h.progress.health(), health);

    // --- Assert: Admin (persistent) survived — admin-gated call still works ---
    h.progress.pause_contract();
    assert!(h.progress.health().paused);
    h.progress.unpause_contract();
    assert!(!h.progress.health().paused);

    // --- Assert: the re-wired verification link works — advance_level (which
    // requires the link) still advances a player one tier post-upgrade. ---
    h.progress.advance_level(&h.verifier, &1u64, &2u32);
    assert_eq!(
        h.progress.get_level(&1u64),
        ProgressLevel::PerformanceMilestones
    );

    // --- Re-wire the remaining two instance-storage links (plain overwrites for
    // progress — no guard flags). These must succeed. ---
    h.progress
        .set_registration_contract(&Address::generate(&h.env));
    h.progress
        .set_scout_access_contract(&Address::generate(&h.env));
}

/// Deliberately-broken upgrade — proves the harness is not a no-op.
///
/// The operator forgets to re-verify the instance `Paused` flag after the
/// upgrade and the contract is left paused. The harness's post-upgrade
/// functional check — a state-changing `advance_level` call — then panics with
/// `ContractPaused`, catching the skipped re-verification step instead of
/// letting a half-configured contract through.
#[test]
#[should_panic]
fn test_progress_broken_upgrade_left_paused_is_caught() {
    let h = setup();
    seed(&h);

    rehearse_upgrade(&h);

    // Simulate a botched re-verification: the contract is (still) paused.
    h.progress.pause_contract();

    // Post-upgrade functional check — must not silently succeed while paused.
    h.progress.advance_level(&h.verifier, &1u64, &2u32);
}
