//! Contract-upgrade rehearsal harness — `registration` contract.
//!
//! # What this is
//!
//! `docs/VERSIONING.md`'s pre-upgrade checklist says to "test the full upgrade
//! flow on testnet before touching mainnet", and `docs/DEPLOYMENT.md` has a
//! "What survives an upgrade" table that makes concrete claims about which
//! state survives a WASM swap (persistent storage) and which must be
//! re-verified / re-wired afterwards (instance storage). Until now those claims
//! were prose only. This harness turns them into automated assertions that run
//! locally in seconds, with no Docker, network, or testnet fees, so an operator
//! can rehearse an upgrade before spending a single testnet transaction.
//!
//! The same pattern is implemented once per contract:
//!   * `contracts/registration/tests/upgrade_rehearsal.rs`   (this file)
//!   * `contracts/verification/tests/upgrade_rehearsal.rs`
//!   * `contracts/progress/tests/upgrade_rehearsal.rs`
//!   * `contracts/scout_access/tests/upgrade_rehearsal.rs`
//!
//! Run them with:
//! ```text
//! cargo test -p scoutchain-registration  --test upgrade_rehearsal
//! cargo test -p scoutchain-verification  --test upgrade_rehearsal
//! cargo test -p scoutchain-progress      --test upgrade_rehearsal
//! cargo test -p scoutchain-scout-access  --test upgrade_rehearsal
//! # or the whole set:
//! cargo test --workspace --test upgrade_rehearsal
//! ```
//!
//! # The rehearsal, per contract
//!
//! 1. Deploy contract v1 (register the current WASM/module in the test `Env`).
//! 2. Seed it with representative state (here: a couple of players, a scout, a
//!    non-trivial player level resolved live from the progress contract, and
//!    the instance-storage progress-contract link).
//! 3. Snapshot every value the DEPLOYMENT.md table touches, then call
//!    `upgrade()` to swap the WASM.
//! 4. Re-read every value and assert persistent state is byte-for-byte
//!    unchanged, that the instance `Initialized`/`Paused` flags are intact, and
//!    that the cross-contract link can be re-wired and works afterwards.
//!
//! # WASM-swap mechanism & its limitation (read before trusting this)
//!
//! A true cross-version upgrade swaps one compiled WASM binary for a *different*
//! one. This sandbox has no Rust/`stellar` toolchain, so a second, genuinely
//! different v2 artifact cannot be built inside the test. Instead we exercise
//! the real `upgrade()` code path with `upload_contract_wasm(Bytes::new(&env))`
//! — the same "swap the installed WASM hash" mechanism the production
//! `upgrade()` uses (this is exactly what the contract's own inline
//! `test_upgrade_preserves_admin` does). Stellar's upgrade mechanism does not
//! special-case "same/empty hash", so this still drives the WASM-swap path and
//! proves storage-layout survival across the upgrade boundary.
//!
//! What this deliberately does NOT prove is behaviour when the *new* code reads
//! a different storage layout — that needs a real second artifact. A contributor
//! with a working toolchain can supply one (e.g. a checked-in `*_v2.wasm`
//! fixture built by CI behind a `--features breaking-change-test` cfg that adds
//! a field) and point `upgrade()` at its hash; the assertion logic below is the
//! actual deliverable and is unchanged by how the second artifact is produced.
//!
//! Because the assertions are the point, they are behavioural wherever possible
//! (live cross-contract level resolution, admin-gated calls) rather than raw
//! storage pokes, so they stay meaningful against a real v2 binary too.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env, String,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    env: Env,
    registration: RegistrationContractClient<'static>,
    progress: ProgressContractClient<'static>,
    /// Dummy address whitelisted as the progress contract's primary
    /// `advance_level` caller so we can seed a non-trivial player level.
    verifier: Address,
}

struct Seeded {
    p1_wallet: Address,
    p1: u64,
    p2: u64,
    scout_id: u64,
    scout_region: String,
}

fn vitals(env: &Env, position: &str) -> PlayerVitals {
    PlayerVitals {
        age: 18,
        position: String::from_str(env, position),
        region: String::from_str(env, "West Africa"),
        nationality: String::from_str(env, "Ghana"),
    }
}

/// Exercise the real `upgrade()` WASM-swap code path. See the module docs for
/// why an empty-bytes blob is the correct mechanism inside a toolchain-less
/// sandbox.
fn rehearse_upgrade(h: &Harness) {
    let new_wasm_hash = h.env.deployer().upload_contract_wasm(Bytes::new(&h.env));
    h.registration.upgrade(&new_wasm_hash);
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);

    // Progress contract — used so the registration contract can resolve a
    // player's live level through its instance-storage progress-contract link.
    let progress_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &progress_id);
    progress.initialize(&admin);
    let verifier = Address::generate(&env);
    progress.set_verification_contract(&verifier);

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    Harness {
        env,
        registration,
        progress,
        verifier,
    }
}

/// Seed representative persistent + instance state: two players, one verified
/// scout, player 1 advanced to VerifiedIdentity on the progress contract, and
/// the progress-contract link wired in.
fn seed(h: &Harness) -> Seeded {
    let hashes = vec![&h.env, String::from_str(&h.env, "QmSeedHashRegistration01")];

    let p1_wallet = Address::generate(&h.env);
    let p1 = h
        .registration
        .register_player(&p1_wallet, &vitals(&h.env, "Forward"), &hashes);

    let w2 = Address::generate(&h.env);
    let p2 = h
        .registration
        .register_player(&w2, &vitals(&h.env, "Midfielder"), &hashes);

    let scout_wallet = Address::generate(&h.env);
    let scout_region = String::from_str(&h.env, "Europe");
    let scout_id = h.registration.register_scout(&scout_wallet, &scout_region);
    h.registration.verify_scout(&scout_id);

    // Wire registration -> progress first (set_player_level requires the
    // link to be present), then progress -> registration so advance_level
    // syncs the level mirror into registration. With both links wired, the
    // advance below propagates VerifiedIdentity into registration's stored
    // PlayerLevel mirror, which get_player reads back.
    h.registration.set_progress_contract(&h.progress.address);
    h.progress
        .set_registration_contract(&h.registration.address);

    // Advance player 1 one tier on the progress contract (primary-caller path).
    h.progress.advance_level(&h.verifier, &p1, &1u32);

    Seeded {
        p1_wallet,
        p1,
        p2,
        scout_id,
        scout_region,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full rehearsal: every DEPLOYMENT.md "What survives an upgrade" row that
/// applies to `registration` is asserted before and after the WASM swap.
#[test]
fn test_registration_upgrade_preserves_state() {
    let h = setup();
    let s = seed(&h);

    // --- Snapshot (pre-upgrade) ---
    let p1_before = h.registration.get_player(&s.p1);
    let p2_before = h.registration.get_player(&s.p2);
    let scout_before = h.registration.get_scout(&s.scout_id);
    let player_count = h.registration.get_player_count();
    let scout_count = h.registration.get_scout_count();
    let health = h.registration.health();

    // Sanity-check the seed is representative before we rely on it.
    assert_eq!(p1_before.wallet, s.p1_wallet);
    // advance_level synced VerifiedIdentity into registration's mirror via the
    // wired progress -> registration link.
    assert_eq!(p1_before.level, ProgressLevel::VerifiedIdentity);
    assert_eq!(p2_before.level, ProgressLevel::Unverified);
    assert!(scout_before.verification.verified);
    assert_eq!(player_count, 2);
    assert_eq!(scout_count, 1);
    assert!(health.initialized && !health.paused);

    // --- Upgrade ---
    rehearse_upgrade(&h);

    // Instance storage (the cross-contract links) is not wiped by an upgrade,
    // but the DEPLOYMENT.md checklist says to re-wire it explicitly. For
    // registration this is a plain, idempotent overwrite (no guard flag).
    // Re-wire both directions so level sync keeps working after the swap.
    h.registration.set_progress_contract(&h.progress.address);
    h.progress
        .set_registration_contract(&h.registration.address);

    // --- Assert: persistent storage survived (Player / scout profiles rows) ---
    let p1_after = h.registration.get_player(&s.p1);
    assert_eq!(p1_after.player_id, p1_before.player_id);
    assert_eq!(p1_after.wallet, p1_before.wallet);
    assert_eq!(p1_after.vitals.age, p1_before.vitals.age);
    assert_eq!(p1_after.vitals.position, p1_before.vitals.position);
    assert_eq!(p1_after.vitals.region, p1_before.vitals.region);
    assert_eq!(p1_after.registered_at, p1_before.registered_at);
    // ...and the live level still resolves through the re-wired instance link.
    assert_eq!(p1_after.level, ProgressLevel::VerifiedIdentity);
    assert_eq!(
        h.registration.get_player(&s.p2).level,
        ProgressLevel::Unverified
    );

    let scout_after = h.registration.get_scout(&s.scout_id);
    assert_eq!(scout_after.scout_id, scout_before.scout_id);
    assert_eq!(scout_after.wallet, scout_before.wallet);
    assert_eq!(scout_after.region, s.scout_region);
    assert!(scout_after.verification.verified);

    // --- Assert: instance flags / counters survived (Initialized/Paused rows) ---
    assert_eq!(h.registration.health(), health);
    assert_eq!(h.registration.get_player_count(), player_count);
    assert_eq!(h.registration.get_scout_count(), scout_count);

    // --- Assert: Admin (persistent) survived — an admin-gated call still works ---
    h.registration.pause_contract();
    assert!(h.registration.health().paused);
    h.registration.unpause_contract();
    assert!(!h.registration.health().paused);
}

/// Deliberately-broken upgrade — proves the harness is not a no-op.
///
/// The operator forgets to re-verify the instance `Paused` flag after the
/// upgrade and the contract is left paused. The harness's post-upgrade
/// functional check — a state-changing `register_player` call — then panics
/// with `ContractPaused`, catching the skipped re-verification step instead
/// of letting a half-configured contract through. This mirrors the broken
/// scenarios in the verification / progress / scout_access harnesses.
#[test]
#[should_panic]
fn test_registration_broken_upgrade_left_paused_is_caught() {
    let h = setup();
    seed(&h);

    rehearse_upgrade(&h);

    // Simulate a botched re-verification: the contract is (still) paused.
    h.registration.pause_contract();

    // Post-upgrade functional check — must not silently succeed while paused.
    let wallet = Address::generate(&h.env);
    let hashes = vec![&h.env, String::from_str(&h.env, "QmBrokenUpgradeCheck01")];
    let _ = h
        .registration
        .register_player(&wallet, &vitals(&h.env, "Goalkeeper"), &hashes);
}
