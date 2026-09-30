/// Integration test: scout_access ↔ progress cross-contract wiring.
///
/// Deploys ProgressContract and ScoutAccessContract in the same Env, wires
/// them together, and verifies that log_trial_offer actually advances the
/// player's level in the progress contract to EliteTier.
///
/// These tests would silently pass (incorrectly) with the old hand-rolled
/// mock client in scout_access — they require the real #[contractclient]
/// implementation that makes genuine cross-contract calls.
use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

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

struct Harness {
    env: Env,
    xlm: Address,
    progress: ProgressContractClient<'static>,
    scout_access: ScoutAccessContractClient<'static>,
    verification: VerificationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);

    // Deploy and initialize the verification contract (the primary
    // whitelisted caller of advance_level).
    let ver_id = env.register(VerificationContract, ());
    let verification = VerificationContractClient::new(&env, &ver_id);
    verification.initialize(&admin);

    // Deploy and initialize the progress contract.
    let progress_id = env.register(ProgressContract, ());
    let progress = ProgressContractClient::new(&env, &progress_id);
    progress.initialize(&admin);
    progress.set_verification_contract(&ver_id);

    // Create the XLM token used by scout_access.
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    // Deploy and initialize the scout_access contract.
    let sa_id = env.register(ScoutAccessContract, ());
    let scout_access = ScoutAccessContractClient::new(&env, &sa_id);
    scout_access.initialize(&admin, &xlm, &default_fees());

    // Wire scout_access → progress so log_trial_offer can call advance_level.
    scout_access.set_progress_contract(&progress_id);
    // Whitelist scout_access as the secondary caller of advance_level.
    progress.set_scout_access_contract(&sa_id);

    Harness {
        env,
        xlm,
        progress,
        scout_access,
        verification,
    }
}

/// Advance a player by `levels` tiers directly in the progress contract,
/// using the verification contract's address as the (primary) caller.
fn advance_player(h: &Harness, player_id: u64, levels: u32) {
    for i in 1..=levels {
        h.progress
            .advance_level(&h.verification.address, &player_id, &i);
    }
}

/// Register a validator and approve one milestone for `player_id` on the
/// verification contract, so that advance_level's secondary-caller
/// milestone_ref validation (used by log_trial_offer) has a valid count
/// to check against. `evidence_hash` must be a unique, valid CID per call
/// (the verification contract rejects duplicate evidence hashes).
fn approve_milestone(h: &Harness, player_id: u64, evidence_hash: &str) {
    let validator = Address::generate(&h.env);
    h.verification.register_validator(
        &validator,
        &String::from_str(&h.env, "UEFA-B-License"),
        &String::from_str(&h.env, "Default Academy"),
        &soroban_sdk::Vec::new(&h.env),
    );
    h.verification.approve_milestone(
        &validator,
        &player_id,
        &String::from_str(&h.env, "scored"),
        &String::from_str(&h.env, evidence_hash),
        &None,
    );
}

/// Mint enough XLM for an Elite subscription, subscribe, and pay to contact
/// `player_id` — log_trial_offer requires a prior contact record.
fn subscribe_elite(h: &Harness, scout: &Address, player_id: u64) {
    StellarAssetClient::new(&h.env, &h.xlm).mint(scout, &10_000_000i128);
    h.scout_access.subscribe(scout, &SubscriptionTier::Elite);
    h.scout_access.pay_to_contact(scout, &player_id);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Happy path: log_trial_offer advances a player from PerformanceMilestones
/// to EliteTier via a real cross-contract call to the progress contract.
#[test]
fn test_log_trial_offer_advances_player_to_elite_tier() {
    let h = setup();
    let player_id: u64 = 1;
    let scout = Address::generate(&h.env);

    // PerformanceMilestones is level 2 — two advance_level calls needed.
    advance_player(&h, player_id, 2);
    assert_eq!(
        h.progress.get_level(&player_id),
        ProgressLevel::PerformanceMilestones,
    );

    subscribe_elite(&h, &scout, player_id);

    // The trial offer's milestone_ref (index 1) must validate against the
    // verification contract's milestone count for this player.
    approve_milestone(
        &h,
        player_id,
        "QmghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ1",
    );

    let index = h.scout_access.log_trial_offer(
        &scout,
        &player_id,
        &String::from_str(&h.env, "QmdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWX"),
    );

    assert_eq!(index, 1);
    assert_eq!(h.scout_access.get_trial_count(&player_id), 1);

    // The two-step flow: the *player* confirms the offer, which is what
    // triggers the cross-contract advance_level call.
    let player_wallet = Address::generate(&h.env);
    h.scout_access
        .confirm_trial_offer(&player_wallet, &player_id, &index, &None);

    // The cross-contract call must have advanced the player to EliteTier.
    assert_eq!(h.progress.get_level(&player_id), ProgressLevel::EliteTier);
}

/// Edge case: when the player is already at EliteTier, log_trial_offer must
/// silently ignore the AlreadyAtMaxLevel error from the progress contract
/// and still record the offer and return Ok.
#[test]
fn test_log_trial_offer_already_at_max_level_is_silent() {
    let h = setup();
    let player_id: u64 = 99;
    let scout = Address::generate(&h.env);

    // Advance all three tiers — player starts at EliteTier.
    advance_player(&h, player_id, 3);
    assert_eq!(h.progress.get_level(&player_id), ProgressLevel::EliteTier);

    subscribe_elite(&h, &scout, player_id);

    // Each log_trial_offer call validates its milestone_ref (the trial
    // counter index) against the verification contract's milestone count.
    approve_milestone(
        &h,
        player_id,
        "QmqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ123456789a",
    );
    approve_milestone(
        &h,
        player_id,
        "QmtuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ123456789abcd",
    );

    // First offer: advance_level returns AlreadyAtMaxLevel; must be ignored.
    let index = h.scout_access.log_trial_offer(
        &scout,
        &player_id,
        &String::from_str(&h.env, "QmjkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ1234"),
    );
    assert_eq!(index, 1);
    assert_eq!(h.progress.get_level(&player_id), ProgressLevel::EliteTier);

    // Advance past the 24h per-(scout, player) trial-offer cooldown so the
    // second offer from the same scout is not rate-limited.
    h.env.ledger().with_mut(|l| l.timestamp += 86_400 + 1);

    // Second offer: same behavior — offer recorded, level unchanged.
    let index2 = h.scout_access.log_trial_offer(
        &scout,
        &player_id,
        &String::from_str(&h.env, "QmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ1234567"),
    );
    assert_eq!(index2, 2);
    assert_eq!(h.scout_access.get_trial_count(&player_id), 2);
    assert_eq!(h.progress.get_level(&player_id), ProgressLevel::EliteTier);
}

/// Issue #425 — TrialCounter increments correctly across multiple log_trial_offer calls.
///
/// Two different Elite scouts each log one trial offer for the same player.
/// Using distinct scouts avoids the 24-hour per-(scout, player) cooldown so
/// both offers can be recorded in the same test environment without advancing
/// the ledger timestamp.
///
/// Acceptance criteria:
///   - First log_trial_offer returns trial index 1
///   - Second log_trial_offer (different scout) returns trial index 2
///   - get_trial_offer(player_id, 1) returns the first offer with correct scout
///   - get_trial_offer(player_id, 2) returns the second offer with correct scout
///   - get_trial_count returns 2 after both calls
#[test]
fn test_trial_counter_increments_across_two_scouts() {
    let h = setup();
    let player_id: u64 = 42;

    // Advance the player to PerformanceMilestones (level 2) so the first
    // log_trial_offer can advance them to EliteTier via the progress contract.
    advance_player(&h, player_id, 2);
    assert_eq!(
        h.progress.get_level(&player_id),
        ProgressLevel::PerformanceMilestones,
    );

    // Each log_trial_offer call validates its milestone_ref (the trial
    // counter index) against the verification contract's milestone count.
    approve_milestone(
        &h,
        player_id,
        "QmwxyzABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdefg",
    );
    approve_milestone(
        &h,
        player_id,
        "QmzABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdefghij",
    );

    // Scout A — first trial offer
    let scout_a = Address::generate(&h.env);
    subscribe_elite(&h, &scout_a, player_id);

    let hash_a = String::from_str(&h.env, "QmCDEFGHJKLMNPQRSTUVWXYZ123456789abcdefghijkmn");
    let index_a = h
        .scout_access
        .log_trial_offer(&scout_a, &player_id, &hash_a);

    // First call must return index 1
    assert_eq!(index_a, 1, "first trial offer must be assigned index 1");
    // Counter is now 1
    assert_eq!(h.scout_access.get_trial_count(&player_id), 1);

    // Player advances to EliteTier only after the player confirms the offer
    // (two-step flow: log_trial_offer escrows, confirm_trial_offer advances).
    let player_wallet = Address::generate(&h.env);
    h.scout_access
        .confirm_trial_offer(&player_wallet, &player_id, &index_a, &None);
    assert_eq!(h.progress.get_level(&player_id), ProgressLevel::EliteTier);

    // Scout B — second trial offer for the same player.
    // A different scout is used to avoid the 24-hour per-(scout, player) cooldown.
    let scout_b = Address::generate(&h.env);
    subscribe_elite(&h, &scout_b, player_id);

    let hash_b = String::from_str(&h.env, "QmFGHJKLMNPQRSTUVWXYZ123456789abcdefghijkmnopq");
    let index_b = h
        .scout_access
        .log_trial_offer(&scout_b, &player_id, &hash_b);

    // Second call must return index 2
    assert_eq!(index_b, 2, "second trial offer must be assigned index 2");
    // Counter is now 2
    assert_eq!(h.scout_access.get_trial_count(&player_id), 2);

    // Retrieve the first offer and verify its fields
    let offer_1 = h.scout_access.get_trial_offer(&player_id, &1u32);
    assert_eq!(offer_1.player_id, player_id);
    assert_eq!(offer_1.scout, scout_a);
    assert_eq!(offer_1.details_hash, hash_a);

    // Retrieve the second offer and verify its fields
    let offer_2 = h.scout_access.get_trial_offer(&player_id, &2u32);
    assert_eq!(offer_2.player_id, player_id);
    assert_eq!(offer_2.scout, scout_b);
    assert_eq!(offer_2.details_hash, hash_b);
}
