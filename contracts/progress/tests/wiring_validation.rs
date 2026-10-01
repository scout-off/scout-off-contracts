//! Wiring validation tests for `advance_level` and `reset_player_level` — Issue #1409.
//!
//! These tests verify that level changes require a wired registration contract
//! and properly validate player existence.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressError};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, String, Vec,
};

/// Helper to create valid player vitals.
fn valid_vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Forward"),
        region: String::from_str(env, "EU"),
        nationality: String::from_str(env, "FR"),
    }
}

/// Helper to create valid IPFS hashes.
fn one_hash(env: &Env) -> Vec<String> {
    let mut v = Vec::new(env);
    v.push_back(String::from_str(env, "bafytestcid"));
    v
}

/// Set up a full stack: registration, progress, and verification contracts.
/// Returns (registration_client, progress_client, verification_client, admin, verification_id)
fn setup_full_stack() -> (
    RegistrationContractClient<'static>,
    ProgressContractClient<'static>,
    VerificationContractClient<'static>,
    Address,
    Address,
) {
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

    // Wire progress -> verification (primary caller)
    progress.set_verification_contract(&ver_id);
    // Wire progress -> registration (required for level changes)
    progress.set_registration_contract(&reg_id);
    // Wire registration -> progress so set_player_level require_auth succeeds
    registration.set_progress_contract(&prog_id);

    (registration, progress, verification, admin, ver_id)
}

#[test]
fn test_advance_level_rejected_when_registration_unwired() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let prog_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &prog_id);
    progress.initialize(&admin);

    let ver_id = Address::generate(&env);
    progress.set_verification_contract(&ver_id);

    // Registration contract is NOT wired
    let player_id: u64 = 1;

    let result = progress.try_advance_level(&ver_id, &player_id, &1u32);

    assert!(
        matches!(result, Err(Ok(ProgressError::RegistrationNotConfigured))),
        "advance_level must fail with RegistrationNotConfigured when registration is unwired: {result:?}"
    );

    // Level and history must be unchanged
    assert_eq!(progress.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(progress.get_history_count(&player_id), 0);
}

#[test]
fn test_reset_player_level_rejected_when_registration_unwired() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let prog_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &prog_id);
    progress.initialize(&admin);

    let player_id: u64 = 1;

    let result = progress.try_reset_player_level(&player_id, &ProgressLevel::VerifiedIdentity);

    assert!(
        matches!(result, Err(Ok(ProgressError::RegistrationNotConfigured))),
        "reset_player_level must fail with RegistrationNotConfigured when registration is unwired: {result:?}"
    );

    // Level and history must be unchanged
    assert_eq!(progress.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(progress.get_history_count(&player_id), 0);
}

#[test]
fn test_advance_level_rejected_for_unknown_player_when_wired() {
    let (registration, progress, verification, _admin, ver_id) = setup_full_stack();

    // Player 999 does NOT exist in registration
    let unknown_player_id: u64 = 999;

    let result = progress.try_advance_level(&ver_id, &unknown_player_id, &1u32);

    assert!(
        matches!(result, Err(Ok(ProgressError::PlayerNotRegistered))),
        "advance_level must fail with PlayerNotRegistered for unknown player: {result:?}"
    );

    // Level and history must be unchanged
    assert_eq!(
        progress.get_level(&unknown_player_id),
        ProgressLevel::Unverified
    );
    assert_eq!(progress.get_history_count(&unknown_player_id), 0);
}

#[test]
fn test_reset_player_level_rejected_for_unknown_player_when_wired() {
    let (registration, progress, _verification, _admin, ver_id) = setup_full_stack();

    // Player 999 does NOT exist in registration
    let unknown_player_id: u64 = 999;

    let result = progress.try_reset_player_level(&unknown_player_id, &ProgressLevel::VerifiedIdentity);

    assert!(
        matches!(result, Err(Ok(ProgressError::PlayerNotRegistered))),
        "reset_player_level must fail with PlayerNotRegistered for unknown player: {result:?}"
    );

    // Level and history must be unchanged
    assert_eq!(
        progress.get_level(&unknown_player_id),
        ProgressLevel::Unverified
    );
    assert_eq!(progress.get_history_count(&unknown_player_id), 0);
}

#[test]
fn test_advance_level_succeeds_for_registered_player_when_wired() {
    let (registration, progress, _verification, _admin, ver_id) = setup_full_stack();

    // Register a player in the registration contract
    let wallet = Address::generate(&registration.env());
    let player_id = registration
        .register_player(&wallet, &valid_vitals(&registration.env()), &one_hash(&registration.env()))
        .unwrap();

    // Now advance_level should succeed
    let result = progress.try_advance_level(&ver_id, &player_id, &1u32);

    assert!(result.is_ok(), "advance_level must succeed for registered player: {result:?}");
    assert_eq!(result.unwrap(), ProgressLevel::VerifiedIdentity);

    // Level and history should be updated
    assert_eq!(progress.get_level(&player_id), ProgressLevel::VerifiedIdentity);
    assert_eq!(progress.get_history_count(&player_id), 1);
}

#[test]
fn test_reset_player_level_succeeds_for_registered_player_when_wired() {
    let (registration, progress, _verification, _admin, ver_id) = setup_full_stack();

    // Register a player in the registration contract
    let wallet = Address::generate(&registration.env());
    let player_id = registration
        .register_player(&wallet, &valid_vitals(&registration.env()), &one_hash(&registration.env()))
        .unwrap();

    // First advance the player
    progress.advance_level(&ver_id, &player_id, &1u32);

    // Now reset should succeed
    let result = progress.try_reset_player_level(&player_id, &ProgressLevel::Unverified);

    assert!(result.is_ok(), "reset_player_level must succeed for registered player: {result:?}");

    // Level and history should be updated
    assert_eq!(progress.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(progress.get_history_count(&player_id), 2); // advance + reset
}