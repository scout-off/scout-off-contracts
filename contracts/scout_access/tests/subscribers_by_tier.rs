//! Tests for `get_subscribers_by_tier` — issue #1700.
//!
//! The view returns the wallets stored in `TierSubscribers(tier)`, which is
//! maintained by `write_subscription` on every subscribe / upgrade / renewal.

mod common;

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

const SUB_DURATION: u64 = 30 * 24 * 60 * 60;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: SUB_DURATION,
        pro_contact_limit: 50,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
    }
}

struct Harness {
    env: Env,
    xlm: Address,
    client: ScoutAccessContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &id);
    client.initialize(&admin, &xlm, &default_fees());
    common::wire_registration(&env, Some(&client), None);

    Harness { env, xlm, client }
}

/// Generate a scout funded for several subscriptions.
fn funded_scout(h: &Harness) -> Address {
    let scout = Address::generate(&h.env);
    StellarAssetClient::new(&h.env, &h.xlm).mint(&scout, &50_000_000i128);
    scout
}

#[test]
fn test_subscribers_by_tier_empty_tier_returns_empty() {
    let h = setup();
    assert_eq!(h.client.get_subscribers_by_tier(&SubscriptionTier::Basic).len(), 0);
    assert_eq!(h.client.get_subscribers_by_tier(&SubscriptionTier::Pro).len(), 0);
    assert_eq!(h.client.get_subscribers_by_tier(&SubscriptionTier::Elite).len(), 0);
}

#[test]
fn test_subscribers_by_tier_basic_lists_scout_under_basic_only() {
    let h = setup();
    let scout = funded_scout(&h);
    h.client.subscribe(&scout, &SubscriptionTier::Basic);

    let basic = h.client.get_subscribers_by_tier(&SubscriptionTier::Basic);
    assert_eq!(basic.len(), 1);
    assert_eq!(basic.get(0).unwrap(), scout);
    assert!(!h.client.get_subscribers_by_tier(&SubscriptionTier::Pro).contains(&scout));
    assert!(!h.client.get_subscribers_by_tier(&SubscriptionTier::Elite).contains(&scout));
}

#[test]
fn test_subscribers_by_tier_upgrade_moves_scout_from_basic_to_pro() {
    let h = setup();
    let scout = funded_scout(&h);
    h.client.subscribe(&scout, &SubscriptionTier::Basic);
    h.client.subscribe(&scout, &SubscriptionTier::Pro);

    assert!(!h.client.get_subscribers_by_tier(&SubscriptionTier::Basic).contains(&scout));
    let pro = h.client.get_subscribers_by_tier(&SubscriptionTier::Pro);
    assert_eq!(pro.len(), 1);
    assert_eq!(pro.get(0).unwrap(), scout);
    assert_eq!(h.client.get_subscription(&scout).tier, SubscriptionTier::Pro);
}

#[test]
fn test_subscribers_by_tier_resubscribe_same_tier_does_not_duplicate() {
    let h = setup();
    let scout = funded_scout(&h);
    h.client.subscribe(&scout, &SubscriptionTier::Basic);
    h.client.subscribe(&scout, &SubscriptionTier::Basic);

    let basic = h.client.get_subscribers_by_tier(&SubscriptionTier::Basic);
    assert_eq!(basic.len(), 1);
    assert_eq!(basic.get(0).unwrap(), scout);
}

/// Pins current behaviour: an expired subscription is NOT pruned from the
/// tier index, so the scout is still listed. See
/// https://github.com/scout-off/scout-off-contracts/issues/1634 (tier lists
/// are never pruned). Update this test when pruning lands.
#[test]
fn test_subscribers_by_tier_expired_subscription_still_listed() {
    let h = setup();
    let scout = funded_scout(&h);
    h.client.subscribe(&scout, &SubscriptionTier::Basic);

    h.env.ledger().with_mut(|l| l.timestamp += SUB_DURATION + 1);

    assert!(h.client.get_subscribers_by_tier(&SubscriptionTier::Basic).contains(&scout));
}
