//! Tests for function-scoped pausing of pay_to_contact (#1056).
//!
//! Mirrors the verification contract's function-scoped pausing pattern (#809)
//! applied to `scout_access.pay_to_contact`.
//!
//! Covers:
//! - pay_to_contact blocked by function-scoped pause
//! - Other state-changing functions not affected by function-scoped pause
//! - Interaction between whole-contract pause and function-scoped pause
//! - health() reflecting the function-scoped pause state

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

const CONTACT_FEE: i128 = 100_000;
const BASIC_FEE: i128 = 1_000_000;
const PRO_FEE: i128 = 3_000_000;
const ELITE_FEE: i128 = 7_000_000;
const SUB_DURATION: u64 = 30 * 24 * 3600; // 30 days
const PRO_LIMIT: u32 = 10;
const TRIAL_ESCROW: i128 = 500_000;
const TRIAL_EXPIRY: u64 = 3_600;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: BASIC_FEE,
        pro_sub_stroops: PRO_FEE,
        elite_sub_stroops: ELITE_FEE,
        sub_duration_secs: SUB_DURATION,
        pro_contact_limit: PRO_LIMIT,
        trial_offer_escrow_stroops: TRIAL_ESCROW,
        trial_offer_expiry_secs: TRIAL_EXPIRY,
    }
}

struct Harness {
    env: Env,
    xlm: Address,
    admin: Address,
    contract: ScoutAccessContractClient<'static>,
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
    let contract = ScoutAccessContractClient::new(&env, &id);
    contract.initialize(&admin, &xlm, &default_fees());

    Harness {
        env,
        xlm,
        admin,
        contract,
    }
}

/// Mint XLM and subscribe `scout` to Basic tier.
fn subscribe(h: &Harness, scout: &Address) {
    StellarAssetClient::new(&h.env, &h.xlm).mint(scout, &(BASIC_FEE * 2));
    h.contract.subscribe(scout, &SubscriptionTier::Basic);
}

/// Give an address enough XLM for many operations.
fn fund(h: &Harness, addr: &Address) {
    StellarAssetClient::new(&h.env, &h.xlm).mint(addr, &100_000_000i128);
}

/// Pause only pay_to_contact (function-scoped).
fn pause_function(h: &Harness) {
    assert!(
        h.contract.try_pause_pay_to_contact().is_ok(),
        "pause_pay_to_contact should succeed"
    );
}

/// Unpause pay_to_contact (function-scoped).
fn unpause_function(h: &Harness) {
    assert!(
        h.contract.try_unpause_pay_to_contact().is_ok(),
        "unpause_pay_to_contact should succeed"
    );
}

/// Pause the whole contract.
fn pause_contract(h: &Harness) {
    assert!(
        h.contract.try_pause_contract().is_ok(),
        "pause_contract should succeed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Function-scoped pause for pay_to_contact
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_pay_to_contact_succeeds_when_not_paused() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);
    fund(&h, &scout);

    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(
        result.is_ok(),
        "pay_to_contact should succeed when not paused"
    );
}

#[test]
fn test_pay_to_contact_blocked_by_function_scoped_pause() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);
    fund(&h, &scout);

    // Pause only pay_to_contact
    pause_function(&h);

    // Try to contact a player
    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(
        result.is_err(),
        "pay_to_contact should be blocked by function-scoped pause"
    );
}

#[test]
fn test_pay_to_contact_succeeds_after_unpause() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);
    fund(&h, &scout);

    // Pause pay_to_contact
    pause_function(&h);

    // Unpause pay_to_contact
    unpause_function(&h);

    // Now pay_to_contact should work
    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(
        result.is_ok(),
        "pay_to_contact should succeed after unpause"
    );
}

#[test]
fn test_pay_to_contact_blocked_by_whole_contract_pause() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);
    fund(&h, &scout);

    // Pause entire contract
    pause_contract(&h);

    // Try to contact a player
    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(
        result.is_err(),
        "pay_to_contact should be blocked by whole-contract pause"
    );
}

#[test]
fn test_pay_to_contact_blocked_by_both_pauses() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);
    fund(&h, &scout);

    // Pause both whole-contract and function-scoped
    pause_contract(&h);
    pause_function(&h);

    // Try to contact a player
    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(result.is_err(), "pay_to_contact should be blocked");
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Other functions unaffected by function-scoped pause
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_subscribe_works_when_pay_to_contact_paused() {
    let h = setup();
    let scout = Address::generate(&h.env);

    // Pause only pay_to_contact
    pause_function(&h);

    // subscribe should still work
    subscribe(&h, &scout);
    let sub = h
        .contract
        .try_get_subscription(&scout)
        .expect("get_subscription should succeed")
        .expect("subscribed scout should have a subscription");
    assert_eq!(
        sub.tier,
        SubscriptionTier::Basic,
        "scout should still be subscribable while pay_to_contact is paused"
    );
}

#[test]
fn test_reads_work_when_pay_to_contact_paused() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);

    // Pause only pay_to_contact
    pause_function(&h);

    // Read-only query should work (regardless of pause)
    let sub = h
        .contract
        .try_get_subscription(&scout)
        .expect("get_subscription should succeed")
        .expect("subscribed scout should have a subscription");
    assert_eq!(
        sub.tier,
        SubscriptionTier::Basic,
        "subscription status should still be readable"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Health query reflects function-scoped pause state
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_health_reflects_pay_to_contact_pause_state() {
    let h = setup();

    // Initially not paused on either level
    let health_before = h.contract.health();
    assert!(health_before.initialized, "contract should be initialized");
    assert!(!health_before.paused, "whole-contract should not be paused");
    assert!(
        !health_before.pay_to_contact_paused,
        "pay_to_contact should not be paused initially"
    );

    // Pause only pay_to_contact
    pause_function(&h);

    // Health should now reflect it without pausing the whole contract
    let health_after = h.contract.health();
    assert!(
        !health_after.paused,
        "whole-contract should still not be paused"
    );
    assert!(
        health_after.pay_to_contact_paused,
        "pay_to_contact_paused should be true"
    );

    // Unpause restores it
    unpause_function(&h);
    let health_final = h.contract.health();
    assert!(
        !health_final.pay_to_contact_paused,
        "pay_to_contact_paused should be false after unpause"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Interaction between pause levels
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_whole_contract_pause_overrides_unpause_pay_to_contact() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout);
    fund(&h, &scout);

    // Pause whole contract; function-scoped stays unpaused
    pause_contract(&h);

    // pay_to_contact is still blocked by the whole-contract pause
    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(
        result.is_err(),
        "pay_to_contact should still be blocked by whole-contract pause"
    );
}

#[test]
fn test_function_pause_independent_of_whole_contract_pause() {
    let h = setup();
    let scout = Address::generate(&h.env);

    // Pause only the function
    pause_function(&h);

    // subscribe still works (whole-contract is not paused)
    subscribe(&h, &scout);
    fund(&h, &scout);

    // pay_to_contact is blocked
    let result = h.contract.try_pay_to_contact(&scout, &1u64);
    assert!(result.is_err(), "pay_to_contact should be blocked");
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Events emitted correctly
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_pause_pay_to_contact_emits_event() {
    let h = setup();

    pause_function(&h);

    // The event names are asserted structurally by scripts/check-docs.sh; here
    // we verify the side effect (flag set) and that the call does not panic.
    let health = h.contract.health();
    assert!(health.pay_to_contact_paused, "flag should be set");
}

#[test]
fn test_unpause_pay_to_contact_emits_event() {
    let h = setup();

    pause_function(&h);

    unpause_function(&h);

    let health = h.contract.health();
    assert!(
        !health.pay_to_contact_paused,
        "flag should be cleared after unpause"
    );
}
