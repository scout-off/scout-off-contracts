//! Adversarial tests for issue #811: idempotency and all-or-nothing revert
//! guarantee for `confirm_trial_offer`.
//!
//! These tests prove that a `ProgressCallFailed` error reverts the entire
//! transaction (no partial state committed) and that the new idempotency
//! nonce mechanism makes retries safe.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String,
};

const ESCROW: i128 = 500_000;

fn fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 10_000_000,
        sub_duration_secs: 2_592_000,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: ESCROW,
        trial_offer_expiry_secs: 7_200,
    }
}

/// Valid 46-char CIDv0 (base58btc charset).
fn valid_cid(env: &Env) -> String {
    String::from_str(env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB")
}

/// Deploy + initialize scout_access and a *real* progress contract wired to
/// scout_access (progress is left uninitialized in `setup_unwired_progress`
/// variant below).
fn setup() -> (
    Env,
    Address,
    Address,
    ScoutAccessContractClient<'static>,
    ProgressContractClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let sa_id = env.register(ScoutAccessContract, ());
    let scout_access = ScoutAccessContractClient::new(&env, &sa_id);
    scout_access.initialize(&admin, &xlm, &fees());

    let progress_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &progress_id);

    (env, admin, xlm, scout_access, progress)
}

/// Mint XLM, subscribe as Elite, and pay_to_contact `player_id` — the
/// prerequisite for `log_trial_offer`.
fn subscribe_elite_and_contact(
    env: &Env,
    xlm: &Address,
    scout_access: &ScoutAccessContractClient<'static>,
    scout: &Address,
    player_id: u64,
) {
    StellarAssetClient::new(env, xlm).mint(scout, &20_000_000i128);
    scout_access.subscribe(scout, &SubscriptionTier::Elite);
    scout_access.pay_to_contact(scout, &player_id);
}

#[test]
fn test_confirm_trial_offer_progress_call_failed_reverts_all_state() {
    let (env, _admin, xlm, scout_access, _progress) = setup();
    let scout = Address::generate(&env);
    let player = Address::generate(&env);
    let player_id: u64 = 1;

    subscribe_elite_and_contact(&env, &xlm, &scout_access, &scout, player_id);

    // log_trial_offer succeeds: escrow is written at (player_id, index).
    let index = scout_access.log_trial_offer(&scout, &player_id, &valid_cid(&env));
    assert_eq!(index, 1);

    // scout_access points at a progress contract that has NOT been
    // initialized, so advance_level must fail → ProgressCallFailed.
    scout_access.set_progress_contract(&_progress.address);
    let result = scout_access.try_confirm_trial_offer(&player, &player_id, &index, &None);
    assert!(
        result.is_err(),
        "confirm_trial_offer should fail when progress.advance_level fails"
    );

    // Verify escrow was NOT cleaned up (transaction reverted).
    let escrow = env.as_contract(&scout_access.address, || {
        env.storage()
            .persistent()
            .get::<scoutchain_scout_access::DataKey, scoutchain_scout_access::TrialEscrow>(
                &scoutchain_scout_access::DataKey::TrialEscrow(player_id, index),
            )
    });
    assert!(
        escrow.is_some(),
        "Escrow must still exist when transaction reverts on ProgressCallFailed"
    );
}

#[test]
fn test_confirm_trial_offer_idempotency_nonce_prevents_replay() {
    let (env, admin, xlm, scout_access, progress) = setup();

    // Fully wire progress so the first confirmation succeeds: initialize it
    // and whitelist scout_access as the secondary advance_level caller. The
    // milestone_ref (trial index 1) must validate, so also wire a
    // verification contract with one approved milestone.
    let ver_id = env.register(scoutchain_verification::VerificationContract, ());
    let verification = scoutchain_verification::VerificationContractClient::new(&env, &ver_id);
    verification.initialize(&admin);

    progress.initialize(&admin);
    progress.set_verification_contract(&ver_id);
    scout_access.set_progress_contract(&progress.address);
    progress.set_scout_access_contract(&scout_access.address);

    // Register a validator + approve one milestone for the player so the
    // secondary-caller milestone_ref check passes (index 1 ≤ count 1).
    let validator = Address::generate(&env);
    verification.register_validator(&validator, &String::from_str(&env, "UEFA-B-License"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::Vec::new(&env));
    verification.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "scored"),
        &String::from_str(&env, "QmRhbYsqpiYgUY9KfNCcbfopHPbLnWSVKBpDNs37aZ3kVC"),
        &None,
    );

    let scout = Address::generate(&env);
    let player = Address::generate(&env);
    let player_id: u64 = 1;

    subscribe_elite_and_contact(&env, &xlm, &scout_access, &scout, player_id);

    // Scout logs a trial offer — escrow index is 1 (1-based).
    let index = scout_access.log_trial_offer(&scout, &player_id, &valid_cid(&env));
    assert_eq!(index, 1);

    // First confirmation with a nonce should succeed and clean up the escrow.
    let nonce = String::from_str(&env, "confirm-nonce-1");
    let result =
        scout_access.try_confirm_trial_offer(&player, &player_id, &index, &Some(nonce.clone()));
    assert!(
        result.is_ok(),
        "First confirm_trial_offer with nonce should succeed"
    );

    // Second confirmation with the same nonce returns Ok(()) idempotently,
    // without re-running escrow cleanup or advance_level.
    let result2 = scout_access.try_confirm_trial_offer(&player, &player_id, &index, &Some(nonce));
    assert!(
        result2.is_ok(),
        "Retry with same nonce should succeed idempotently"
    );
}
