//! Adversarial atomicity tests for `progress.advance_level` — Issue #1057.
//!
//! `advance_level` writes a progress-history entry and the new player level,
//! then synchronises the level to the wired registration contract via
//! `try_set_player_level`. If that cross-contract sync fails, Soroban reverts
//! the ENTIRE transaction: no history entry and no level change are committed.
//!
//! These tests convert that guarantee into directly-tested behaviour, in the
//! same spirit as the `confirm_trial_offer` atomicity tests in
//! `contracts/scout_access/tests/adversarial_atomicity.rs`.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressError};
use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

struct Harness {
    registration: RegistrationContractClient<'static>,
    progress: ProgressContractClient<'static>,
    ver_id: Address,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    let prog_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &prog_id);
    progress.initialize(&admin);

    let ver_id = env.register(VerificationContract, ());
    let verification = VerificationContractClient::new(&env, &ver_id);
    verification.initialize(&admin);

    // Whitelist the verification contract as the primary advance_level caller.
    progress.set_verification_contract(&ver_id);
    // Wire progress -> registration so advance_level attempts a real sync.
    progress.set_registration_contract(&reg_id);
    // Wire registration's ProgressContract to the real progress contract so the
    // sync call's `require_auth` succeeds. We still force a sync *failure* by
    // never seeding the player in registration (set_player_level -> PlayerNotFound).
    registration.set_progress_contract(&prog_id);

    Harness {
        registration,
        progress,
        ver_id,
    }
}

/// A failed registration sync must roll the whole `advance_level` transaction
/// back: the level stays Unverified and no history entry is committed.
#[test]
fn test_advance_level_rolls_back_when_registration_sync_fails() {
    let h = setup();
    let player_id: u64 = 1;

    let result = h.progress.try_advance_level(&h.ver_id, &player_id, &0u32);

    assert!(
        matches!(result, Err(Ok(ProgressError::RegistrationCallFailed))),
        "advance_level must fail with RegistrationCallFailed when sync fails: {result:?}"
    );

    // Atomicity: nothing from the rolled-back advance may have persisted.
    assert_eq!(
        h.progress.get_level(&player_id),
        ProgressLevel::Unverified,
        "level must remain Unverified after a rolled-back advance"
    );
    assert_eq!(
        h.progress.get_history_count(&player_id),
        0,
        "no progress history may be committed after a rolled-back advance"
    );
}

/// `advance_level` invoked with no whitelisted verification contract must be
/// rejected and must commit no state (atomic no-op).
#[test]
fn test_advance_level_rejects_when_unconfigured_and_commits_nothing() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let prog_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &prog_id);
    progress.initialize(&admin);

    let caller = Address::generate(&env);
    let player_id: u64 = 1;
    let result = progress.try_advance_level(&caller, &player_id, &0u32);

    assert!(
        matches!(result, Err(Ok(ProgressError::NotInitialized))),
        "unconfigured advance_level must fail with NotInitialized: {result:?}"
    );
    assert_eq!(progress.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(progress.get_history_count(&player_id), 0);
}
