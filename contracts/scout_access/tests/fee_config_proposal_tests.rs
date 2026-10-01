//! Tests for the timelocked propose-then-activate fee config mechanism (#807).
//!
//! Covers:
//! - Proposing fee increases (delayed activation)
//! - Proposing fee decreases (immediate activation)
//! - Activating pending proposals after the delay
//! - Rejecting premature activations
//! - Verifying that business logic uses the active config, never the pending one
//! - Handling overlapping proposals

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
    token::StellarAssetClient,
    Address, Env, IntoVal, Symbol,
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
    scout: Address,
    contract_id: Address,
    client: ScoutAccessContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let scout = Address::generate(&env);

    // Create XLM token
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    // Mint XLM to scout for fee payments
    let token_client = StellarAssetClient::new(&env, &xlm);
    token_client.mint(&scout, &1_000_000_000);

    // Deploy and initialize scout_access
    let contract_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm, &default_fees());

    Harness {
        env,
        admin,
        scout,
        contract_id,
        client,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Proposing fee increases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_propose_fee_config_with_increase() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000; // Increase from 7M to 10M

    // Propose the increase
    h.client.propose_fee_config(&new_config);

    // Verify the active config hasn't changed (still the old fees)
    let active_config = h.client.get_fee_config();
    assert_eq!(active_config.elite_sub_stroops, 7_000_000);
}

#[test]
fn test_propose_fee_config_with_decrease() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 5_000_000; // Decrease from 7M to 5M

    // Propose the decrease
    h.client.propose_fee_config(&new_config);

    // Verify the config was immediately activated
    let active_config = h.client.get_fee_config();
    assert_eq!(active_config.elite_sub_stroops, 5_000_000);
}

#[test]
fn test_propose_fee_config_with_mixed_change() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000; // Increase
    new_config.pro_sub_stroops = 2_000_000; // Decrease (still ≥ MIN_SUB_FEE_STROOPS)

    // Propose the mixed change
    h.client.propose_fee_config(&new_config);

    // Since there's an increase, it should be pending (not activated)
    let active_config = h.client.get_fee_config();
    assert_eq!(active_config.elite_sub_stroops, 7_000_000); // Still old value
    assert_eq!(active_config.pro_sub_stroops, 3_000_000); // Still old value
}

#[test]
fn test_propose_fee_config_invalid_validation() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.basic_sub_stroops = 0; // Invalid: zero fee

    // Propose should reject due to validation
    let result = h.client.try_propose_fee_config(&new_config);
    assert!(result.is_err());
}

#[test]
fn test_propose_fee_config_requires_admin() {
    // Fresh env WITHOUT mock_all_auths so the admin gate can actually fail.
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let admin = Address::generate(&env);
    let contract_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &contract_id);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    // Mock ONLY the admin's initialize auth; no auths are mocked afterwards,
    // so the require_admin check inside propose_fee_config fails for any caller.
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

    // Propose should reject due to missing admin auth.
    let result = client.try_propose_fee_config(&default_fees());
    assert!(result.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Activating pending proposals
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_activate_fee_config_after_delay() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000; // Increase

    // Propose the increase at timestamp 1_000_000
    h.client.propose_fee_config(&new_config);

    // Verify still not active
    let active = h.client.get_fee_config();
    assert_eq!(active.elite_sub_stroops, 7_000_000);

    // Advance time by 7 days + 1 second
    h.env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (7 * 24 * 60 * 60) + 1;
    });

    // Activate the proposal
    h.client.activate_fee_config();

    // Verify it's now active
    let active = h.client.get_fee_config();
    assert_eq!(active.elite_sub_stroops, 10_000_000);
}

#[test]
fn test_activate_fee_config_before_delay_fails() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000; // Increase

    h.client.propose_fee_config(&new_config);

    // Try to activate immediately (no time advanced)
    let result = h.client.try_activate_fee_config();
    assert!(result.is_err());
}

#[test]
fn test_activate_fee_config_almost_but_not_quite_ready() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000;

    h.client.propose_fee_config(&new_config);

    // Advance time by 7 days - 1 second
    h.env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (7 * 24 * 60 * 60) - 1;
    });

    // Should still fail
    let result = h.client.try_activate_fee_config();
    assert!(result.is_err());
}

#[test]
fn test_activate_fee_config_with_no_pending() {
    let h = setup();

    // Try to activate when no proposal exists
    let result = h.client.try_activate_fee_config();
    assert!(result.is_err());
}

#[test]
fn test_activate_fee_config_requires_admin() {
    // Fresh env WITHOUT mock_all_auths so the admin gate can actually fail.
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let admin = Address::generate(&env);
    let contract_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &contract_id);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000;

    // Mock ONLY the admin's auth for the calls that must succeed.
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

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "propose_fee_config",
            args: soroban_sdk::vec![&env, new_config.clone().into_val(&env)],
            sub_invokes: &[],
        },
    }]);
    client.propose_fee_config(&new_config);

    // Advance time
    env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (7 * 24 * 60 * 60) + 1;
    });

    // Try to activate with no auth mocked — must be rejected.
    let result = client.try_activate_fee_config();
    assert!(result.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Subscribe uses active config during proposal window
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_subscribe_during_proposal_window_uses_old_fee() {
    let h = setup();

    // Propose an increase
    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000; // Increase from 7M to 10M

    h.client.propose_fee_config(&new_config);

    // Subscribe while proposal is pending
    h.client.subscribe(&h.scout, &SubscriptionTier::Elite);

    // The scout should have been charged the OLD elite_sub_stroops (7M)
    let accumulated = h.client.get_accumulated_fees();
    assert_eq!(accumulated, 7_000_000);
}

#[test]
fn test_subscribe_after_activation_uses_new_fee() {
    let h = setup();

    // Propose an increase
    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000;

    h.client.propose_fee_config(&new_config);

    // Advance time by 7 days + 1 second
    h.env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (7 * 24 * 60 * 60) + 1;
    });

    // Activate
    h.client.activate_fee_config();

    // Subscribe after activation
    h.client.subscribe(&h.scout, &SubscriptionTier::Elite);

    // The scout should be charged the NEW elite_sub_stroops (10M)
    let accumulated = h.client.get_accumulated_fees();
    assert_eq!(accumulated, 10_000_000);
}

#[test]
fn test_pay_to_contact_during_proposal_window_uses_old_fee() {
    let h = setup();

    // First subscribe the scout
    h.client.subscribe(&h.scout, &SubscriptionTier::Pro);

    // Contact a player to establish eligibility
    let player_id = 1u64;
    h.client.pay_to_contact(&h.scout, &player_id);

    let accumulated_before_proposal = h.client.get_accumulated_fees();

    // Propose a fee increase
    let mut new_config = default_fees();
    new_config.contact_fee_stroops = 500_000; // Increase from 100k

    h.client.propose_fee_config(&new_config);

    // Contact another player while proposal is pending
    let player_id_2 = 2u64;
    h.client.pay_to_contact(&h.scout, &player_id_2);

    // The second contact should have been charged the OLD contact_fee_stroops (100k)
    let accumulated_after = h.client.get_accumulated_fees();
    assert_eq!(accumulated_after - accumulated_before_proposal, 100_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Overlapping proposals
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_propose_fee_config_with_existing_pending_proposal() {
    let h = setup();

    let mut config1 = default_fees();
    config1.elite_sub_stroops = 10_000_000;

    let mut config2 = default_fees();
    config2.elite_sub_stroops = 15_000_000;

    // Propose the first increase
    h.client.propose_fee_config(&config1);

    // Try to propose a second increase while first is pending
    let result = h.client.try_propose_fee_config(&config2);
    assert!(result.is_err()); // Should reject due to existing pending proposal
}

#[test]
fn test_propose_after_activate_allows_new_proposal() {
    let h = setup();

    let mut config1 = default_fees();
    config1.elite_sub_stroops = 10_000_000;

    h.client.propose_fee_config(&config1);

    // Advance time and activate
    h.env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (7 * 24 * 60 * 60) + 1;
    });

    h.client.activate_fee_config();

    // Now propose a new config
    let mut config2 = default_fees();
    config2.elite_sub_stroops = 15_000_000;

    h.client.propose_fee_config(&config2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: update_fee_config's delay-bypass is distinguishable in the event
// stream from activate_fee_config's delay-respecting activation (#1055)
// ─────────────────────────────────────────────────────────────────────────────

/// `update_fee_config` bypasses the 7-day propose/activate delay by design
/// (see docs/FEE_CONFIG_PROPOSAL_DESIGN.md, "Coexist"). It must flag that
/// bypass in the event stream: alongside the existing `fee_config_updated`
/// event, it must also emit `fee_config_delay_bypassed` — an additive event
/// that leaves `fee_config_updated`'s own topics/data unchanged for existing
/// consumers.
#[test]
fn test_update_fee_config_emits_delay_bypassed_event() {
    let h = setup();

    let old_config = default_fees();
    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 9_000_000;

    h.client.update_fee_config(&new_config);

    // `events().all()` reflects only the most recent invocation.
    let events = h.env.events().all();
    let expected = soroban_sdk::vec![
        &h.env,
        (
            h.contract_id.clone(),
            (Symbol::new(&h.env, "fee_config_updated"), h.admin.clone()).into_val(&h.env),
            (old_config.clone(), new_config.clone()).into_val(&h.env),
        ),
        (
            h.contract_id.clone(),
            (
                Symbol::new(&h.env, "fee_config_delay_bypassed"),
                h.admin.clone()
            )
                .into_val(&h.env),
            (old_config.clone(), new_config.clone()).into_val(&h.env),
        ),
    ];
    assert_eq!(
        events, expected,
        "update_fee_config must emit fee_config_updated AND fee_config_delay_bypassed"
    );
}

/// `activate_fee_config`, by contrast, respects the full 7-day delay and must
/// emit only `fee_config_updated` — never `fee_config_delay_bypassed`. This
/// is the actual distinguishing signal an indexer/auditor relies on: a
/// `fee_config_updated` event with no accompanying `fee_config_delay_bypassed`
/// (and no accompanying `fee_config_proposed`, which would instead indicate
/// propose_fee_config's own immediate-decrease shortcut) means the change
/// went through the full delay.
#[test]
fn test_activate_fee_config_does_not_emit_delay_bypassed_event() {
    let h = setup();

    let old_config = default_fees();
    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 10_000_000; // Increase — requires the delay

    h.client.propose_fee_config(&new_config);

    h.env.ledger().with_mut(|l| {
        l.timestamp = 1_000_000 + (7 * 24 * 60 * 60) + 1;
    });

    h.client.activate_fee_config();

    // `events().all()` reflects only the most recent invocation (activate).
    let events = h.env.events().all();
    let expected = soroban_sdk::vec![
        &h.env,
        (
            h.contract_id.clone(),
            (Symbol::new(&h.env, "fee_config_updated"), h.admin.clone()).into_val(&h.env),
            (old_config.clone(), new_config.clone()).into_val(&h.env),
        ),
    ];
    assert_eq!(
        events, expected,
        "activate_fee_config must emit fee_config_updated and never fee_config_delay_bypassed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Backwards compatibility with update_fee_config
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_update_fee_config_still_works() {
    let h = setup();

    let mut new_config = default_fees();
    new_config.elite_sub_stroops = 12_000_000; // Arbitrary new value

    // Old update_fee_config should still work
    h.client.update_fee_config(&new_config);

    // Should be immediately active
    let active = h.client.get_fee_config();
    assert_eq!(active.elite_sub_stroops, 12_000_000);
}

#[test]
fn test_update_fee_config_and_propose_coexist() {
    let h = setup();

    // Use old update_fee_config
    let mut config1 = default_fees();
    config1.elite_sub_stroops = 9_000_000;

    h.client.update_fee_config(&config1);

    // Verify it's active
    let active = h.client.get_fee_config();
    assert_eq!(active.elite_sub_stroops, 9_000_000);

    // Now use new propose_fee_config to propose an increase
    let mut config2 = default_fees();
    config2.elite_sub_stroops = 12_000_000;

    h.client.propose_fee_config(&config2);

    // Old config should still be active
    let active = h.client.get_fee_config();
    assert_eq!(active.elite_sub_stroops, 9_000_000);
}
