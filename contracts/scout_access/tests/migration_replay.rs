//! Tests for issue #1038: verified cross-contract state migration replay.
//!
//! Covers `admin_seed_subscription`, `admin_seed_contact`,
//! `admin_seed_trial_offer`, and `admin_seed_auto_renew` in the scout_access
//! contract, plus migration-window management.

use scoutchain_scout_access::{
    ContactRecord, FeeConfig, FeeConfigHistoryEntry, ScoutAccessContract,
    ScoutAccessContractClient, ScoutAccessError, Subscription, SubscriptionTier, TrialEscrow,
    TrialOffer,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

fn make_fee_config() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: 1_000_000,
        basic_sub_stroops: 10_000_000,
        pro_sub_stroops: 30_000_000,
        elite_sub_stroops: 70_000_000,
        sub_duration_secs: 30 * 24 * 3600,
        trial_offer_escrow_stroops: 5_000_000,
        trial_offer_expiry_secs: 7 * 24 * 3600,
        pro_contact_limit: 10,
    }
}

fn setup() -> (Env, ScoutAccessContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &id);
    let admin = Address::generate(&env);

    // Register native XLM token for fee config.
    let xlm_id = env.register_stellar_asset_contract_v2(admin.clone());
    let xlm_addr = xlm_id.address();

    client.initialize(&admin, &xlm_addr, &make_fee_config());
    (env, client, admin)
}

// ── Migration window lifecycle ────────────────────────────────────────────────

#[test]
fn test_scout_access_migration_window_lifecycle() {
    let (_env, client, _admin) = setup();
    assert!(!client.migration_window_is_open());
    client.open_migration_window();
    assert!(client.migration_window_is_open());
    client.close_migration_window();
    assert!(!client.migration_window_is_open());
}

// ── admin_seed_subscription ───────────────────────────────────────────────────

#[test]
fn test_seed_subscription_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    let scout = Address::generate(&env);
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Pro,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    let result = client.try_admin_seed_subscription(&sub);
    assert_eq!(result, Err(Ok(ScoutAccessError::MigrationNotActive)));
}

#[test]
fn test_seed_subscription_happy_path() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Elite,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    client.admin_seed_subscription(&sub);

    let stored = client.get_subscription(&scout);
    assert_eq!(stored.tier, SubscriptionTier::Elite);
    assert_eq!(stored.expires_at, sub.expires_at);
    assert_eq!(stored.subscribed_at, sub.subscribed_at);
}

#[test]
fn test_seed_subscription_idempotent() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Basic,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    client.admin_seed_subscription(&sub);

    // Replay — must be no-op.
    let result = client.try_admin_seed_subscription(&sub);
    assert!(result.is_ok(), "identical subscription replay must succeed");

    // TierSubscribers must not contain duplicates.
    let subs = client.get_subscribers_by_tier(&SubscriptionTier::Basic);
    assert_eq!(
        subs.len(),
        1u32,
        "TierSubscribers must not contain duplicates"
    );
}

#[test]
fn test_seed_subscription_conflict_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Pro,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    client.admin_seed_subscription(&sub);

    // Same scout, different tier → conflict.
    let conflict = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Elite,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    let result = client.try_admin_seed_subscription(&conflict);
    assert_eq!(result, Err(Ok(ScoutAccessError::SubscriptionAlreadyExists)));
}

#[test]
fn test_seed_subscription_populates_expiry_bucket() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    // Keep the expiry close to the epoch so `get_expiring_subscriptions` only
    // scans a handful of day buckets (its scan is O(cutoff_day)).
    let expires_at = 2u64 * 86_400;
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Pro,
        expires_at,
        subscribed_at: 0,
    };
    client.admin_seed_subscription(&sub);

    // The expiry bucket must be populated by the seed.
    let expiring = client.get_expiring_subscriptions(&(expires_at + 1), &50u32);
    assert!(
        expiring.iter().any(|s| s.scout == scout),
        "ExpiryBucket must be populated by seed"
    );
}

// ── admin_seed_contact ────────────────────────────────────────────────────────

#[test]
fn test_seed_contact_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    let scout = Address::generate(&env);
    let contact = ContactRecord {
        player_id: 1,
        scout: scout.clone(),
        contacted_at: 1_700_000_000,
    };
    let result = client.try_admin_seed_contact(&contact);
    assert_eq!(result, Err(Ok(ScoutAccessError::MigrationNotActive)));
}

#[test]
fn test_seed_contact_happy_path() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let contact = ContactRecord {
        player_id: 1,
        scout: scout.clone(),
        contacted_at: 1_700_000_000,
    };
    client.admin_seed_contact(&contact);

    let scout_contacts = client.get_scout_contacts(&scout);
    assert!(scout_contacts.iter().any(|id| id == 1u64));

    let player_scouts = client.get_player_contacts(&1u64);
    assert!(player_scouts.iter().any(|a| a == scout));
}

#[test]
fn test_seed_contact_idempotent() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let contact = ContactRecord {
        player_id: 1,
        scout: scout.clone(),
        contacted_at: 1_700_000_000,
    };
    client.admin_seed_contact(&contact);

    // Replay 4×.
    for _ in 0..4 {
        let r = client.try_admin_seed_contact(&contact);
        assert!(r.is_ok());
    }

    // No duplicates in any index.
    let sc = client.get_scout_contacts(&scout);
    assert_eq!(sc.len(), 1u32, "ScoutContacts must not have duplicates");
    let pc = client.get_player_contacts(&1u64);
    assert_eq!(pc.len(), 1u32, "PlayerContacts must not have duplicates");
}

#[test]
fn test_seed_multiple_contacts_for_same_scout() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);

    for player_id in 1u64..=3 {
        let contact = ContactRecord {
            player_id,
            scout: scout.clone(),
            contacted_at: 1_700_000_000 + player_id,
        };
        client.admin_seed_contact(&contact);
    }

    let sc = client.get_scout_contacts(&scout);
    assert_eq!(sc.len(), 3u32);
}

// ── admin_seed_trial_offer ────────────────────────────────────────────────────

#[test]
fn test_seed_trial_offer_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    let scout = Address::generate(&env);
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    let result = client.try_admin_seed_trial_offer(&1u64, &1u32, &offer, &None);
    assert_eq!(result, Err(Ok(ScoutAccessError::MigrationNotActive)));
}

#[test]
fn test_seed_trial_offer_happy_path_with_escrow() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    let escrow = TrialEscrow {
        amount: 5_000_000,
        expires_at: 1_700_000_000 + 7 * 86_400,
    };
    client.admin_seed_trial_offer(&1u64, &1u32, &offer, &Some(escrow));

    assert_eq!(client.get_trial_count(&1u64), 1u32);

    let sto = client.get_scout_trial_offers(&scout);
    assert_eq!(sto.len(), 1u32);
}

#[test]
fn test_seed_trial_offer_happy_path_without_escrow() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    let result = client.try_admin_seed_trial_offer(&1u64, &1u32, &offer, &None);
    assert!(result.is_ok());
    assert_eq!(client.get_trial_count(&1u64), 1u32);
}

#[test]
fn test_seed_trial_offer_idempotent() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    client.admin_seed_trial_offer(&1u64, &1u32, &offer, &None);

    for _ in 0..3 {
        let r = client.try_admin_seed_trial_offer(&1u64, &1u32, &offer, &None);
        assert!(r.is_ok());
    }
    assert_eq!(client.get_trial_count(&1u64), 1u32);
    let sto = client.get_scout_trial_offers(&scout);
    assert_eq!(sto.len(), 1u32, "no duplicates in ScoutTrialOffers");
}

#[test]
fn test_seed_trial_offer_conflict_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    client.admin_seed_trial_offer(&1u64, &1u32, &offer, &None);

    let conflict = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 9_999_999_999, // different
    };
    let result = client.try_admin_seed_trial_offer(&1u64, &1u32, &conflict, &None);
    assert_eq!(result, Err(Ok(ScoutAccessError::TrialOfferAlreadyExists)));
}

#[test]
fn test_seed_trial_offer_out_of_order_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    // Skip index 1, try index 2 directly.
    let result = client.try_admin_seed_trial_offer(&1u64, &2u32, &offer, &None);
    assert_eq!(result, Err(Ok(ScoutAccessError::TrialOfferNotFound)));
}

// ── admin_seed_auto_renew ─────────────────────────────────────────────────────

#[test]
fn test_seed_auto_renew_rejected_when_window_closed() {
    let (env, client, _admin) = setup();
    let scout = Address::generate(&env);
    let result = client.try_admin_seed_auto_renew(&scout, &true);
    assert_eq!(result, Err(Ok(ScoutAccessError::MigrationNotActive)));
}

#[test]
fn test_seed_auto_renew_enabled() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    client.admin_seed_auto_renew(&scout, &true);
    assert!(client.get_auto_renew(&scout));
}

#[test]
fn test_seed_auto_renew_disabled() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    client.admin_seed_auto_renew(&scout, &false);
    assert!(!client.get_auto_renew(&scout));
}

#[test]
fn test_seed_auto_renew_idempotent_true() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    client.admin_seed_auto_renew(&scout, &true);
    let result = client.try_admin_seed_auto_renew(&scout, &true);
    assert!(result.is_ok());
    assert!(client.get_auto_renew(&scout));
}

#[test]
fn test_seed_auto_renew_idempotent_false() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    client.admin_seed_auto_renew(&scout, &false);
    let result = client.try_admin_seed_auto_renew(&scout, &false);
    assert!(result.is_ok());
    assert!(!client.get_auto_renew(&scout));
}

#[test]
fn test_seed_auto_renew_conflict_rejected() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    client.admin_seed_auto_renew(&scout, &true);
    let result = client.try_admin_seed_auto_renew(&scout, &false);
    assert_eq!(result, Err(Ok(ScoutAccessError::AutoRenewAlreadyExists)));
}

// ── Security: seeding after window close fails ────────────────────────────────

#[test]
fn test_all_seeds_rejected_after_window_close() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    // Seed one item.
    client.admin_seed_auto_renew(&scout, &true);
    // Close the window.
    client.close_migration_window();

    // Auto-renew: rejected.
    let r1 = client.try_admin_seed_auto_renew(&scout, &true);
    assert_eq!(r1, Err(Ok(ScoutAccessError::MigrationNotActive)));

    // Subscription: rejected.
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Basic,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    let r2 = client.try_admin_seed_subscription(&sub);
    assert_eq!(r2, Err(Ok(ScoutAccessError::MigrationNotActive)));

    // Contact: rejected.
    let contact = ContactRecord {
        player_id: 1,
        scout: scout.clone(),
        contacted_at: 1_700_000_000,
    };
    let r3 = client.try_admin_seed_contact(&contact);
    assert_eq!(r3, Err(Ok(ScoutAccessError::MigrationNotActive)));

    // Trial offer: rejected.
    let offer = TrialOffer {
        player_id: 1,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    let r4 = client.try_admin_seed_trial_offer(&1u64, &1u32, &offer, &None);
    assert_eq!(r4, Err(Ok(ScoutAccessError::MigrationNotActive)));
}

// ── Full state parity check ───────────────────────────────────────────────────

#[test]
fn test_full_migration_sequence_parity() {
    let (env, client, _admin) = setup();
    client.open_migration_window();
    let scout = Address::generate(&env);
    let player_id = 42u64;

    // Subscription.
    let sub = Subscription {
        scout: scout.clone(),
        tier: SubscriptionTier::Elite,
        expires_at: 1_700_000_000 + 30 * 86_400,
        subscribed_at: 1_700_000_000,
    };
    client.admin_seed_subscription(&sub);

    // Contact.
    let contact = ContactRecord {
        player_id,
        scout: scout.clone(),
        contacted_at: 1_700_000_000,
    };
    client.admin_seed_contact(&contact);

    // Trial offer with escrow.
    let offer = TrialOffer {
        player_id,
        scout: scout.clone(),
        details_hash: String::from_str(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB"),
        logged_at: 1_700_000_000,
    };
    let escrow = TrialEscrow {
        amount: 5_000_000,
        expires_at: 1_700_000_000 + 7 * 86_400,
    };
    client.admin_seed_trial_offer(&player_id, &1u32, &offer, &Some(escrow.clone()));

    // AutoRenew.
    client.admin_seed_auto_renew(&scout, &true);

    // Verify parity.
    assert_eq!(
        client.get_subscription(&scout).tier,
        SubscriptionTier::Elite
    );
    assert_eq!(client.get_scout_contacts(&scout).len(), 1u32);
    assert_eq!(client.get_trial_count(&player_id), 1u32);
    assert_eq!(client.get_trial_escrow(&player_id, &1u32), Some(escrow));
    assert!(client.get_auto_renew(&scout));

    let historical = FeeConfigHistoryEntry {
        config: make_fee_config(),
        updated_at: 1_699_000_000,
    };
    client.admin_seed_fee_config(
        &make_fee_config(),
        &soroban_sdk::vec![&env, historical.clone()],
    );
    assert_eq!(client.get_fee_config(), make_fee_config());
    assert_eq!(
        client.get_fee_config_history(),
        soroban_sdk::vec![&env, historical]
    );

    // Close the migration window.
    client.close_migration_window();
    assert!(!client.migration_window_is_open());
}
