/// Integration test: registration <-> progress cross-contract link.
///
/// Deploys RegistrationContract and ProgressContract in the same Env, wires
/// them together via set_registration_contract / set_progress_contract, and
/// verifies that advance_level's push-sync into registration's
/// set_player_level actually updates registration's level-indexed storage
/// (used by filter_players) — not just the independent live-read path
/// registration.get_player already uses via its own resolve_level() ->
/// progress.get_level() cross-call.
///
/// That distinction matters: get_player().level would report the correct
/// level even if the push-sync (set_player_level) were completely broken,
/// because it re-derives the level from progress on every read regardless.
/// filter_players, by contrast, depends entirely on the PlayersByLevel /
/// PlayersByLevelRegion index buckets that only set_player_level maintains.
/// So this file exercises filter_players specifically to prove the sync
/// itself is doing real, load-bearing work — confirming the link described
/// in docs/DEPLOYMENT.md's wiring list is not vestigial.
///
/// Mirrors the structure of
/// contracts/scout_access/tests/integration_trial_offer_flow.rs.
use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

struct Harness {
    env: Env,
    registration: RegistrationContractClient<'static>,
    progress: ProgressContractClient<'static>,
    // Stands in for the verification contract, which is the whitelisted
    // caller of advance_level. The verification <-> progress link itself is
    // out of scope here — it's already covered by progress's own unit tests
    // and doesn't need a real verification contract deployed for this file
    // to exercise the registration <-> progress link in isolation.
    verification: Address,
}

fn dummy_vitals(env: &Env, region: &str) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Midfielder"),
        region: String::from_str(env, region),
        nationality: String::from_str(env, "Nigeria"),
    }
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    let prog_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &prog_id);
    progress.initialize(&admin);

    let verification = Address::generate(&env);
    progress.set_verification_contract(&verification);

    Harness {
        env,
        registration,
        progress,
        verification,
    }
}

fn wire_registration_link(h: &Harness) {
    h.progress.set_registration_contract(&h.registration.address);
    h.registration.set_progress_contract(&h.progress.address);
}

/// AC1: advance_level on progress must sync into registration such that
/// filter_players — which depends on set_player_level having maintained the
/// level index buckets — finds the player under their new level.
#[test]
fn test_advance_level_syncs_into_registration_level_index() {
    let h = setup();
    wire_registration_link(&h);

    let region = "West Africa";
    let wallet = Address::generate(&h.env);
    let vitals = dummy_vitals(&h.env, region);
    let hashes = vec![&h.env, String::from_str(&h.env, "QmTest")];
    let player_id = h.registration.register_player(&wallet, &vitals, &hashes);

    let region_str = String::from_str(&h.env, region);
    let no_position_filter = String::from_str(&h.env, "");

    // Before any advance, the player only sits in the Unverified bucket —
    // filtering by min_level=VerifiedIdentity must not find them yet.
    let before = h.registration.filter_players(
        &region_str,
        &no_position_filter,
        &ProgressLevel::VerifiedIdentity,
        &0u32,
        &50u32,
    );
    assert_eq!(before.profiles.len(), 0);

    // Advance the player one tier in the progress contract. This must
    // trigger progress.advance_level -> registration.set_player_level,
    // moving the player from the Unverified bucket into the
    // VerifiedIdentity bucket.
    h.progress.advance_level(&h.verification, &player_id, &1u32);

    let after = h.registration.filter_players(
        &region_str,
        &no_position_filter,
        &ProgressLevel::VerifiedIdentity,
        &0u32,
        &50u32,
    );
    assert_eq!(after.profiles.len(), 1);
    assert_eq!(after.profiles.get(0).unwrap().player_id, player_id);

    // The independent live-read path agrees too.
    let profile = h.registration.get_player(&player_id);
    assert_eq!(profile.level, ProgressLevel::VerifiedIdentity);
}

/// AC2: set_player_level must only be callable by the address registered via
/// set_progress_contract. This exercises that guard against the *real*,
/// fully-wired progress contract address (rather than an arbitrary
/// placeholder), confirming the auth check holds up in the actual deployment
/// topology this file sets up.
#[test]
fn test_set_player_level_rejects_non_progress_contract_caller() {
    let h = setup();
    wire_registration_link(&h);

    let wallet = Address::generate(&h.env);
    let vitals = dummy_vitals(&h.env, "Europe");
    let hashes = vec![&h.env, String::from_str(&h.env, "QmTest")];
    let player_id = h.registration.register_player(&wallet, &vitals, &hashes);

    // Clear mocked auths so the registered progress contract address's
    // require_auth() inside set_player_level cannot be satisfied by any
    // caller.
    h.env.mock_auths(&[]);
    let result = h
        .registration
        .try_set_player_level(&player_id, &ProgressLevel::VerifiedIdentity);
    assert!(result.is_err());
}

/// AC3: the specific failure mode when the link is only half-wired. Progress
/// is configured with registration's address (set_registration_contract),
/// but registration was never told progress's address in return
/// (set_progress_contract) — a realistic misconfiguration, since
/// docs/DEPLOYMENT.md lists both directions as separate wiring steps.
/// registration.set_player_level then has no configured ProgressContract to
/// check require_auth() against and rejects the call, which must propagate
/// back through progress as RegistrationCallFailed rather than silently
/// succeeding.
#[test]
fn test_advance_level_returns_registration_call_failed_when_registration_side_unwired() {
    let h = setup();

    h.progress.set_registration_contract(&h.registration.address);
    // Deliberately skip: h.registration.set_progress_contract(...)

    let wallet = Address::generate(&h.env);
    let vitals = dummy_vitals(&h.env, "Europe");
    let hashes = vec![&h.env, String::from_str(&h.env, "QmTest")];
    let player_id = h.registration.register_player(&wallet, &vitals, &hashes);

    let result = h
        .progress
        .try_advance_level(&h.verification, &player_id, &1u32);

    assert!(result.is_err());
    // ProgressError isn't part of the progress crate's public API (by
    // design — see its errors module), so it can't be named/matched on
    // directly from this external test crate. Its Debug representation
    // still surfaces the variant name, which is enough to confirm this is
    // specifically RegistrationCallFailed and not some other failure.
    assert!(
        format!("{result:?}").contains("RegistrationCallFailed"),
        "expected RegistrationCallFailed, got: {result:?}"
    );
}

/// Documents that the registration link is optional, backward-compatible
/// configuration, not a required dependency: with neither
/// set_registration_contract nor set_progress_contract ever called,
/// advance_level must not attempt (and therefore cannot fail) a sync it was
/// never configured to perform. registration.get_player still resolves the
/// correct level on its own via a live cross-call to progress.get_level,
/// entirely independent of set_player_level ever having run — see the
/// module-level comment for why that distinction is the whole point of this
/// file.
#[test]
fn test_advance_level_succeeds_without_registration_contract_configured() {
    let h = setup();

    let wallet = Address::generate(&h.env);
    let vitals = dummy_vitals(&h.env, "Europe");
    let hashes = vec![&h.env, String::from_str(&h.env, "QmTest")];
    let player_id = h.registration.register_player(&wallet, &vitals, &hashes);

    let level = h.progress.advance_level(&h.verification, &player_id, &1u32);
    assert_eq!(level, ProgressLevel::VerifiedIdentity);

    let profile = h.registration.get_player(&player_id);
    assert_eq!(profile.level, ProgressLevel::VerifiedIdentity);
}
