//! Contract-upgrade rehearsal harness — `verification` contract.
//!
//! See `contracts/registration/tests/upgrade_rehearsal.rs` for the full write-up
//! of what this family of harnesses is, why it exists (turning the prose
//! "What survives an upgrade" table in `docs/DEPLOYMENT.md` into automated
//! assertions), and the WASM-swap mechanism and its limitation (a genuinely
//! different v2 artifact cannot be built in this toolchain-less sandbox, so the
//! real `upgrade()` code path is driven with an empty-bytes WASM blob — the same
//! mechanism the contract's own inline upgrade test uses).
//!
//! Run: `cargo test -p scoutchain-verification --test upgrade_rehearsal`.
//!
//! ## Contract-specific quirk exercised here
//!
//! Unlike the other contracts, `verification` guards its progress-contract link
//! with a one-time `ProgressContractSet` instance flag: `set_progress_contract`
//! returns `AlreadyConfigured` if called a second time. Because instance storage
//! (including that flag) survives an `upgrade()`, the post-upgrade re-wiring step
//! MUST use `update_progress_contract`, not `set_progress_contract`
//! (`docs/DEPLOYMENT.md`, "For `verification`, re-wire the progress contract
//! link"). The happy-path test re-wires with `update_progress_contract`; the
//! deliberately-broken test proves an operator who reaches for
//! `set_progress_contract` is caught.

use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, Env, String,
};

// Valid 46-char CIDv0 evidence hashes (base58btc, no 0/O/I/l). Each milestone
// needs a globally-unique evidence hash, so the trailing character differs.
const CID_1: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const CID_2: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqC";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    env: Env,
    verification: VerificationContractClient<'static>,
}

struct Seeded {
    validator: Address,
}

fn rehearse_upgrade(h: &Harness) {
    let new_wasm_hash = h.env.deployer().upload_contract_wasm(Bytes::new(&h.env));
    h.verification.upgrade(&new_wasm_hash);
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let id = env.register(VerificationContract, ());
    let verification = VerificationContractClient::new(&env, &id);
    verification.initialize(&admin);

    Harness { env, verification }
}

/// Seed a validator plus one approved milestone for each of two players.
///
/// The progress-contract link is wired LAST, after the approvals: with a link
/// set, `approve_milestone` would try to cross-call `advance_level`, so seeding
/// the milestones first keeps this harness self-contained (no progress crate).
fn seed(h: &Harness) -> Seeded {
    let validator = Address::generate(&h.env);
    h.verification.register_validator(
        &validator,
        &String::from_str(&h.env, "UEFA-B-License"),
        &String::from_str(&h.env, "Default Academy"),
        &soroban_sdk::Vec::new(&h.env),
    );

    h.verification.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&h.env, "scored"),
        &String::from_str(&h.env, CID_1),
        &None,
    );
    h.verification.approve_milestone(
        &validator,
        &2u64,
        &String::from_str(&h.env, "assisted"),
        &String::from_str(&h.env, CID_2),
        &None,
    );

    // Initial progress-contract wiring (sets the one-time ProgressContractSet
    // guard flag in instance storage).
    let progress_link = Address::generate(&h.env);
    h.verification.set_progress_contract(&progress_link);

    Seeded { validator }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full rehearsal for `verification`.
#[test]
fn test_verification_upgrade_preserves_state() {
    let h = setup();
    let s = seed(&h);

    // --- Snapshot (pre-upgrade) ---
    let milestone_before = h.verification.get_milestone(&1u64, &1u32); // Milestone: PartialEq
    let p1_count = h.verification.get_milestone_count(&1u64);
    let validator_before = h.verification.get_validator(&s.validator);
    let total_before = h.verification.get_total_milestone_count(); // instance counter
    let active_validators = h.verification.get_active_validator_count(); // instance counter
    let health = h.verification.health();

    assert_eq!(p1_count, 1);
    assert_eq!(total_before, 2);
    assert_eq!(active_validators, 1);
    assert!(validator_before.active);
    assert!(health.initialized && !health.paused);

    // --- Upgrade ---
    rehearse_upgrade(&h);

    // Correct post-upgrade re-wiring: the ProgressContractSet guard survived the
    // upgrade, so we MUST use update_progress_contract (see module docs). This
    // must succeed without panicking.
    let new_progress_link = Address::generate(&h.env);
    h.verification.update_progress_contract(&new_progress_link);

    // --- Assert: persistent storage survived ---
    // Validator registry row.
    let validator_after = h.verification.get_validator(&s.validator);
    assert_eq!(validator_after.wallet, validator_before.wallet);
    assert_eq!(validator_after.credentials, validator_before.credentials);
    assert!(validator_after.active);
    // Milestone records row (Milestone derives PartialEq — compare whole value).
    assert_eq!(h.verification.get_milestone(&1u64, &1u32), milestone_before);
    assert_eq!(h.verification.get_milestone_count(&1u64), p1_count);
    assert_eq!(h.verification.get_milestone_count(&2u64), 1);

    // --- Assert: instance flags / counters survived ---
    assert_eq!(h.verification.health(), health);
    assert_eq!(h.verification.get_total_milestone_count(), total_before);
    assert_eq!(
        h.verification.get_active_validator_count(),
        active_validators
    );

    // --- Assert: Admin (persistent) survived — admin-gated call still works ---
    h.verification.pause_contract();
    assert!(h.verification.health().paused);
    h.verification.unpause_contract();
    assert!(!h.verification.health().paused);
}

/// Deliberately-broken upgrade — proves the harness is not a no-op.
///
/// After the upgrade the operator reaches for `set_progress_contract` to re-wire
/// the link, forgetting that verification's one-time `ProgressContractSet` guard
/// survived the WASM swap. That call returns `AlreadyConfigured`, which the
/// generated client surfaces as a panic. This is the exact regression the
/// `docs/DEPLOYMENT.md` note warns about; the harness catches it instead of
/// letting a mis-wired upgrade proceed to mainnet.
#[test]
#[should_panic]
fn test_verification_broken_upgrade_wrong_rewire_fn_is_caught() {
    let h = setup();
    let _ = seed(&h);

    rehearse_upgrade(&h);

    // Wrong re-wiring function post-upgrade — must not silently succeed.
    let new_progress_link = Address::generate(&h.env);
    h.verification.set_progress_contract(&new_progress_link);
}
