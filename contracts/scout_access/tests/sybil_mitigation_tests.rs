//! Tests for Sybil-resistant Pro-tier gating (#808).
//!
//! Covers:
//! - Unverified scouts cannot subscribe to Pro tier
//! - Verified scouts can subscribe to Pro tier
//! - Unverified scouts can still get Basic and Elite tiers
//! - Admin can verify scouts
//! - Registration contract wiring

use scoutchain_registration::{RegistrationContract, RegistrationContractClient};
use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::StellarAssetClient,
    Address, Env, IntoVal, String,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

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
    admin: Address,
    scout1: Address,
    scout2: Address,
    xlm: Address,
    registration_client: RegistrationContractClient<'static>,
    scout_access_client: ScoutAccessContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let scout1 = Address::generate(&env);
    let scout2 = Address::generate(&env);

    // Create XLM token
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    // Mint XLM to scouts
    let token_client = StellarAssetClient::new(&env, &xlm);
    token_client.mint(&scout1, &1_000_000_000);
    token_client.mint(&scout2, &1_000_000_000);

    // Deploy registration contract
    let reg_id = env.register(RegistrationContract, ());
    let registration_client = RegistrationContractClient::new(&env, &reg_id);
    registration_client.initialize(&admin);

    // Register scouts in registration contract
    let _scout1_id =
        registration_client.register_scout(&scout1, &String::from_str(&env, "North America"));

    let _scout2_id = registration_client.register_scout(&scout2, &String::from_str(&env, "Europe"));

    // Deploy scout_access contract
    let sa_id = env.register(ScoutAccessContract, ());
    let scout_access_client = ScoutAccessContractClient::new(&env, &sa_id);
    scout_access_client.initialize(&admin, &xlm, &default_fees());

    // Wire registration contract into scout_access
    scout_access_client.set_registration_contract(&reg_id);

    Harness {
        env,
        admin,
        scout1,
        scout2,
        xlm,
        registration_client,
        scout_access_client,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Pro-tier gating
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_unverified_scout_cannot_subscribe_pro() {
    let h = setup();

    // scout1 is unverified by default
    let result = h
        .scout_access_client
        .try_subscribe(&h.scout1, &SubscriptionTier::Pro);
    assert!(
        result.is_err(),
        "Unverified scout should not be able to subscribe to Pro"
    );
}

/// Regression test for #1053 / #808: `subscribe()`'s Pro-tier gate must call
/// through to `registration_contract::get_scout_by_wallet` (see
/// `contracts/scout_access/src/lib.rs`'s `subscribe()`) and reject an
/// unverified scout with the specific `ScoutAccessError::ScoutNotVerified`
/// (code 27) error, and must let a verified scout through — not just fail
/// with *some* error, which the other tests in this file already check less
/// precisely via `is_err()`.
#[test]
fn test_unverified_scout_rejected_with_scout_not_verified_error() {
    use scoutchain_scout_access::ScoutAccessError;

    let h = setup();

    // scout1 is unverified by default: the specific error must be
    // ScoutNotVerified, not just any error.
    let result = h
        .scout_access_client
        .try_subscribe(&h.scout1, &SubscriptionTier::Pro);
    assert_eq!(
        result
            .expect_err("unverified scout must be rejected")
            .expect("contract error"),
        ScoutAccessError::ScoutNotVerified,
        "unverified scout must be rejected specifically with ScoutNotVerified"
    );

    // Verify scout1 via the registration contract, then confirm the same
    // subscribe() call now succeeds.
    let scout1_profile = h.registration_client.get_scout_by_wallet(&h.scout1);
    h.registration_client.verify_scout(&scout1_profile.scout_id);

    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Pro);
}

#[test]
fn test_verified_scout_can_subscribe_pro() {
    let h = setup();

    // Get scout1's ID and verify them
    let scout1_profile = h.registration_client.get_scout_by_wallet(&h.scout1);
    let scout1_id = scout1_profile.scout_id;

    h.registration_client.verify_scout(&scout1_id);

    // Now scout1 should be able to subscribe to Pro
    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Pro);
}

#[test]
fn test_unverified_scout_can_subscribe_basic() {
    let h = setup();

    // Basic tier should work for unverified scouts
    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Basic);
}

#[test]
fn test_unverified_scout_can_subscribe_elite() {
    let h = setup();

    // Elite tier should work for unverified scouts (no gating on Elite)
    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Elite);
}

#[test]
fn test_multiple_scouts_independent_verification() {
    let h = setup();

    // Get scout profiles
    let scout1_profile = h.registration_client.get_scout_by_wallet(&h.scout1);
    let _scout2_profile = h.registration_client.get_scout_by_wallet(&h.scout2);

    // Verify only scout1
    h.registration_client.verify_scout(&scout1_profile.scout_id);

    // scout1 can subscribe to Pro
    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Pro);

    // scout2 (still unverified) cannot subscribe to Pro
    let result = h
        .scout_access_client
        .try_subscribe(&h.scout2, &SubscriptionTier::Pro);
    assert!(
        result.is_err(),
        "unverified scout2 should not subscribe to Pro"
    );

    // scout2 can still get Elite
    h.scout_access_client
        .subscribe(&h.scout2, &SubscriptionTier::Elite);
}

#[test]
fn test_verified_scout_can_renew_pro() {
    let h = setup();

    // Get scout1's ID and verify
    let scout1_profile = h.registration_client.get_scout_by_wallet(&h.scout1);

    h.registration_client.verify_scout(&scout1_profile.scout_id);

    // Subscribe to Pro
    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Pro);

    // Advance time to expire subscription
    h.env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (35 * 24 * 60 * 60);
    });

    // Renew subscription (should still work since scout remains verified)
    h.scout_access_client
        .subscribe(&h.scout1, &SubscriptionTier::Pro);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Registration contract wiring
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_set_registration_contract_requires_admin() {
    // Fresh env WITHOUT mock_all_auths so the admin gate can actually fail.
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let admin = Address::generate(&env);
    let contract_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &contract_id);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    // Mock ONLY the admin's initialize auth; set_registration_contract's
    // require_admin check then fails for any unauthenticated caller.
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "initialize",
            args: soroban_sdk::vec![
                &env,
                admin.to_val(),
                xlm.to_val(),
                default_fees().into_val(&env)
            ],
            sub_invokes: &[],
        },
    }]);
    client.initialize(&admin, &xlm, &default_fees());

    let random_contract = Address::generate(&env);
    let result = client.try_set_registration_contract(&random_contract);
    assert!(
        result.is_err(),
        "non-admin should not be able to wire registration contract"
    );
}

#[test]
fn test_registration_contract_graceful_degradation() {
    let h = setup();
    let scout3 = Address::generate(&h.env);

    // Mint XLM to scout3
    let token_client = StellarAssetClient::new(&h.env, &h.xlm);
    token_client.mint(&scout3, &1_000_000_000);

    // Do NOT wire registration contract; create a new scout_access instance
    let sa_id = h.env.register(ScoutAccessContract, ());
    let sa_client = ScoutAccessContractClient::new(&h.env, &sa_id);
    sa_client.initialize(&h.admin, &h.xlm, &default_fees());

    // Without registration contract wired, Pro subscriptions should be allowed (graceful degradation)
    sa_client.subscribe(&scout3, &SubscriptionTier::Pro);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Sybil attack scenarios
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_multi_wallet_sybil_attempt_blocked() {
    let h = setup();

    // Attacker registers 3 scout wallets
    let attacker1 = Address::generate(&h.env);
    let attacker2 = Address::generate(&h.env);
    let attacker3 = Address::generate(&h.env);

    let token_client = StellarAssetClient::new(&h.env, &h.xlm);
    token_client.mint(&attacker1, &1_000_000_000);
    token_client.mint(&attacker2, &1_000_000_000);
    token_client.mint(&attacker3, &1_000_000_000);

    // Register all 3 in registration contract
    let _id1 = h
        .registration_client
        .register_scout(&attacker1, &String::from_str(&h.env, "Region1"));
    let _id2 = h
        .registration_client
        .register_scout(&attacker2, &String::from_str(&h.env, "Region2"));
    let _id3 = h
        .registration_client
        .register_scout(&attacker3, &String::from_str(&h.env, "Region3"));

    // All 3 are unverified by default
    let result1 = h
        .scout_access_client
        .try_subscribe(&attacker1, &SubscriptionTier::Pro);
    let result2 = h
        .scout_access_client
        .try_subscribe(&attacker2, &SubscriptionTier::Pro);
    let result3 = h
        .scout_access_client
        .try_subscribe(&attacker3, &SubscriptionTier::Pro);

    assert!(
        result1.is_err() && result2.is_err() && result3.is_err(),
        "All unverified wallets should be blocked from Pro tier"
    );

    // Admin can verify one or more, but attacker must convince admin N times
    let attacker1_profile = h.registration_client.get_scout_by_wallet(&attacker1);
    h.registration_client
        .verify_scout(&attacker1_profile.scout_id);

    // Only attacker1 can now subscribe to Pro
    h.scout_access_client
        .subscribe(&attacker1, &SubscriptionTier::Pro);

    // attacker2 and attacker3 still blocked
    let result2 = h
        .scout_access_client
        .try_subscribe(&attacker2, &SubscriptionTier::Pro);
    let result3 = h
        .scout_access_client
        .try_subscribe(&attacker3, &SubscriptionTier::Pro);

    assert!(
        result2.is_err() && result3.is_err(),
        "Unverified wallets should remain blocked"
    );
}

#[test]
fn test_attacker_can_pay_for_elite_instead() {
    let h = setup();

    let attacker = Address::generate(&h.env);
    let token_client = StellarAssetClient::new(&h.env, &h.xlm);
    token_client.mint(&attacker, &1_000_000_000);

    h.registration_client
        .register_scout(&attacker, &String::from_str(&h.env, "AttackerRegion"));

    // Elite tier always works (no verification needed)
    h.scout_access_client
        .subscribe(&attacker, &SubscriptionTier::Elite);

    // This demonstrates that Elite (0.7 XLM unlimited) is still cheaper than
    // 3 Pro wallets (0.9 XLM for 30 contacts), so the mitigation raises friction
    // but doesn't completely prevent an attacker who wants unlimited contacts.
}
