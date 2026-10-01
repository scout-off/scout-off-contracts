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
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, Env, String, Vec,
};

fn valid_vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Forward"),
        region: String::from_str(env, "EU"),
        nationality: String::from_str(env, "FR"),
    }
}

fn one_hash(env: &Env) -> Vec<String> {
    let mut v = Vec::new(env);
    v.push_back(String::from_str(env, "bafytestcid"));
    v
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    env: Env,
    progress: ProgressContractClient<'static>,
    registration: RegistrationContractClient<'static>,
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

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    let id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &id);
    progress.initialize(&admin);

    // Whitelist a primary caller so we can advance player levels. advance_level's
    // primary-caller path does not cross-call the verification contract, so a
    // plain generated address is sufficient here.
    let verifier = Address::generate(&env);
    progress.set_verification_contract(&verifier);
    progress.set_registration_contract(&reg_id);
    registration.set_progress_contract(&id);

    Harness {
        env,
        progress,
        registration,
        verifier,
    }
}

fn seed(h: &Harness) -> (u64, u64, u64) {
    let p1 = {
        let wallet = Address::generate(&h.env);
        h.registration.register_player(&wallet, &valid_vitals(&h.env), &one_hash(&h.env)).unwrap()
    };
    let p2 = {
        let wallet = Address::generate(&h.env);
        h.registration.register_player(&wallet, &valid_vitals(&h.env), &one_hash(&h.env)).unwrap()
    };
    let p3 = {
        let wallet = Address::generate(&h.env);
        h.registration.register_player(&wallet, &valid_vitals(&h.env), &one_hash(&h.env)).unwrap()
    };
    // Player 1 -> VerifiedIdentity (1 advance).
    h.progress.advance_level(&h.verifier, &p1, &1u32);
    // Player 2 -> PerformanceMilestones (2 advances).
    h.progress.advance_level(&h.verifier, &p2, &1u32);
    h.progress.advance_level(&h.verifier, &p2, &2u32);
    // Player 3 -> EliteTier (3 advances).
    h.progress.advance_level(&h.verifier, &p3, &1u32);
    h.progress.advance_level(&h.verifier, &p3, &2u32);
    h.progress.advance_level(&h.verifier, &p3, &3u32);
    (p1, p2, p3)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full rehearsal for `progress`.
#[test]
fn test_progress_upgrade_preserves_state() {
    let h = setup();
    let (p1, p2, p3) = seed(&h);

    // --- Snapshot (pre-upgrade) ---
    let l1 = h.progress.get_level(&p1);
    let l2 = h.progress.get_level(&p2);
    let l3 = h.progress.get_level(&p3);
    let hist1 = h.progress.get_history_count(&p1);
    let hist3 = h.progress.get_history_count(&p3);
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
    assert_eq!(h.progress.get_level(&p1), l1);
    assert_eq!(h.progress.get_level(&p2), l2);
    assert_eq!(h.progress.get_level(&p3), l3);
    assert_eq!(h.progress.get_history_count(&p1), hist1);
    assert_eq!(h.progress.get_history_count(&p3), hist3);

    // --- Assert: instance flags survived ---
    assert_eq!(h.progress.health(), health);

    // --- Assert: Admin (persistent) survived — admin-gated call still works ---
    h.progress.pause_contract();
    assert!(h.progress.health().paused);
    h.progress.unpause_contract();
    assert!(!h.progress.health().paused);

    // --- Assert: the re-wired verification link works — advance_level (which
    // requires the link) still advances a player one tier post-upgrade. ---
    h.progress.advance_level(&h.verifier, &p1, &2u32);
    assert_eq!(
        h.progress.get_level(&p1),
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
/// silently passing.
#[test]
fn test_progress_upgrade_panic_on_missed_paused_flag() {
    let h = setup();
    let (p1, _p2, _p3) = seed(&h);

    // Pause before upgrade
    h.progress.pause_contract();

    rehearse_upgrade(&h);

    // Re-wire verification (instance link) but FORGET to check/clear paused flag.
    h.progress.set_verification_contract(&h.verifier);

    // Post-upgrade functional check — must not silently succeed while paused.
    h.progress.advance_level(&h.verifier, &1u64, &2u32);
}

/// Assert that `upgrade()` emits a `contract_upgraded` event before swapping
/// the WASM, so the event is attributed to the old code version.
#[test]
fn test_progress_upgrade_emits_contract_upgraded_event() {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{symbol_short, IntoVal};

    let h = setup();
    seed(&h);

    let new_wasm_hash = h.env.deployer().upload_contract_wasm(Bytes::new(&h.env));
    h.progress.upgrade(&new_wasm_hash);

    let events = h.env.events().all();
    let found = events.iter().any(|(_, topics, _)| {
        topics.get(0).map_or(false, |first| {
            let expected: soroban_sdk::Val = symbol_short!("contract_upgraded").into_val(&h.env);
            first == expected
        })
    });

    assert!(
        found,
        "expected a 'contract_upgraded' event to be emitted by upgrade()"
    );
}
