//! End-to-end test for GDPR player erasure (issue #1373).
//!
//! Registers a player, creates data in all four contracts, calls the paged
//! purge entrypoints in the recommended order, then asserts that no
//! current-state keys remain readable (except documented tombstones).

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{
    PlayerVitals, RegistrationContract, RegistrationContractClient,
};
use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String, Vec,
};

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
    }
}

fn vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 22,
        position: String::from_str(env, "Midfielder"),
        region: String::from_str(env, "EU"),
        nationality: String::from_str(env, "BR"),
    }
}

fn one_hash(env: &Env) -> Vec<String> {
    let mut v = Vec::new(env);
    v.push_back(String::from_str(env, "bafytest123"));
    v
}

struct Harness {
    env: Env,
    admin: Address,
    xlm: Address,
    registration: RegistrationContractClient<'static>,
    progress: ProgressContractClient<'static>,
    verification: VerificationContractClient<'static>,
    scout_access: ScoutAccessContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);

    // Registration
    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    // Verification
    let ver_id = env.register(VerificationContract, ());
    let verification = VerificationContractClient::new(&env, &ver_id);
    verification.initialize(&admin);

    // Progress
    let prog_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &prog_id);
    progress.initialize(&admin);
    progress.set_verification_contract(&ver_id);
    progress.set_registration_contract(&reg_id);

    // XLM token
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    // ScoutAccess
    let sa_id = env.register(ScoutAccessContract, ());
    let scout_access = ScoutAccessContractClient::new(&env, &sa_id);
    scout_access.initialize(&admin, &xlm, &default_fees());
    scout_access.set_progress_contract(&prog_id);
    progress.set_scout_access_contract(&sa_id);

    // Wire verification → progress
    verification.set_progress_contract(&prog_id);
    // Wire registration → progress (for level sync)
    registration.set_progress_contract(&prog_id);
    progress.set_registration_contract(&reg_id);

    Harness {
        env,
        admin,
        xlm,
        registration,
        progress,
        verification,
        scout_access,
    }
}

#[test]
fn test_full_erasure_leaves_no_readable_state() {
    let h = setup();
    let env = &h.env;

    // --- Step 1: Register player ---
    let player_wallet = Address::generate(env);
    let player_id = h
        .registration
        .register_player(&player_wallet, &vitals(env), &one_hash(env));
    assert_eq!(player_id, 1u64);

    // --- Step 2: Register validator and approve a milestone ---
    let validator_wallet = Address::generate(env);
    h.verification.register_validator(
        &validator_wallet,
        &String::from_str(env, "UEFA B License Coach"),
        &String::from_str(env, "FC Example Academy"),
        &Vec::new(env),
    );
    h.verification.approve_milestone(
        &validator_wallet,
        &player_id,
        &String::from_str(env, "Scored 5 goals in cup"),
        &String::from_str(env, "bafyevidence123"),
        &None,
    );

    // Player should now be at VerifiedIdentity (level 1).
    assert_eq!(
        h.progress.get_level(&player_id),
        ProgressLevel::VerifiedIdentity
    );

    // --- Step 3: Scout subscribes and contacts player ---
    let scout_wallet = Address::generate(env);
    let sa_id = h.scout_access.address.clone();
    let xlm_client = StellarAssetClient::new(env, &h.xlm);
    xlm_client.mint(&scout_wallet, &10_000_000i128);
    // Approve scout_access to spend scout funds.
    let reg_id = h.registration.address.clone();
    h.registration.register_scout(
        &scout_wallet,
        &String::from_str(env, "EU"),
    );
    // Subscribe (Elite tier for contact + trial access)
    h.scout_access.subscribe(&scout_wallet, &SubscriptionTier::Elite);

    // pay_to_contact requires registration wiring for verification in some
    // configurations; we call it here to create a ContactRecord.
    h.scout_access.set_registration_contract(&reg_id);
    h.scout_access.pay_to_contact(&player_id, &scout_wallet);

    // --- Step 4: Run paged erasure (recommended order) ---

    // 4a. Purge scout_access (no outstanding escrows so cursor 0 → done).
    let (cursor, more) = h.scout_access.purge_player_data(&player_id, &0u32);
    assert!(!more, "scout_access purge should complete in one page");
    let _ = cursor; // cursor value only needed for multi-page scenarios

    // 4b. Purge verification.
    let (cursor, more) = h.verification.purge_player_data(&player_id, &0u32);
    assert!(!more, "verification purge should complete in one page");
    let _ = cursor;

    // 4c. Purge progress.
    let (cursor, more) = h.progress.purge_player_data(&player_id, &0u32);
    assert!(!more, "progress purge should complete in one page");
    let _ = cursor;

    // 4d. Deregister from registration.
    h.registration.deregister_player(&player_id);

    // --- Step 5: Assert no current-state keys remain readable ---

    // Registration: profile must be gone.
    let profile_result = h.registration.try_get_player(&player_id);
    assert!(
        profile_result.is_err(),
        "player profile must not be readable after erasure"
    );

    // Progress: level must be gone (returns Unverified as default sentinel,
    // but the storage key itself must not exist — we check that calling
    // get_history_count returns 0).
    assert_eq!(
        h.progress.get_history_count(&player_id),
        0,
        "progress history counter must be 0 after erasure"
    );

    // Verification: milestone count must be 0.
    assert_eq!(
        h.verification.get_milestone_count(&player_id),
        0,
        "milestone counter must be 0 after erasure"
    );
}

#[test]
fn test_erasure_blocks_on_outstanding_escrow() {
    let h = setup();
    let env = &h.env;

    // Register a player.
    let player_wallet = Address::generate(env);
    let player_id = h
        .registration
        .register_player(&player_wallet, &vitals(env), &one_hash(env));

    // Advance player to level 2 so an Elite scout can log a trial offer.
    let validator_wallet = Address::generate(env);
    h.verification.register_validator(
        &validator_wallet,
        &String::from_str(env, "UEFA B License Coach"),
        &String::from_str(env, "FC Example Academy"),
        &Vec::new(env),
    );
    h.verification.approve_milestone(
        &validator_wallet,
        &player_id,
        &String::from_str(env, "Milestone 1"),
        &String::from_str(env, "bafyev001"),
        &None,
    );
    h.verification.approve_milestone(
        &validator_wallet,
        &player_id,
        &String::from_str(env, "Milestone 2"),
        &String::from_str(env, "bafyev002"),
        &None,
    );

    // Set up scout with Elite subscription and funds.
    let scout_wallet = Address::generate(env);
    let xlm_client = StellarAssetClient::new(env, &h.xlm);
    xlm_client.mint(&scout_wallet, &10_000_000i128);
    h.registration.register_scout(
        &scout_wallet,
        &String::from_str(env, "EU"),
    );
    h.scout_access
        .set_registration_contract(&h.registration.address.clone());
    h.scout_access.subscribe(&scout_wallet, &SubscriptionTier::Elite);
    h.scout_access.pay_to_contact(&player_id, &scout_wallet);
    h.scout_access
        .log_trial_offer(&player_id, &scout_wallet, &String::from_str(env, "bafyoffer1"));

    // Attempt purge while escrow is outstanding — must be rejected.
    let result = h
        .scout_access
        .try_purge_player_data(&player_id, &0u32);
    assert!(
        result.is_err(),
        "purge must be blocked while outstanding escrow exists"
    );

    // Advance time past escrow expiry.
    env.ledger().with_mut(|l| l.timestamp += 7_200);

    // Now the escrow is expired — purge should succeed.
    let (_, more) = h.scout_access.purge_player_data(&player_id, &0u32);
    assert!(!more, "purge should complete after escrow expired");
}
