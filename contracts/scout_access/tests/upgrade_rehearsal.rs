//! Contract-upgrade rehearsal harness — `scout_access` contract.
//!
//! See `contracts/registration/tests/upgrade_rehearsal.rs` for the full write-up
//! of what this family of harnesses is, why it exists (turning the prose
//! "What survives an upgrade" table in `docs/DEPLOYMENT.md` into automated
//! assertions), and the WASM-swap mechanism and its limitation (a genuinely
//! different v2 artifact cannot be built in this toolchain-less sandbox, so the
//! real `upgrade()` code path is driven with an empty-bytes WASM blob).
//!
//! Run: `cargo test -p scoutchain-scout-access --test upgrade_rehearsal`.
//!
//! `scout_access` is the richest row-set in the DEPLOYMENT.md table:
//!   * Persistent: subscription records, contact records and scout indexes.
//!   * Instance:   Initialized / Paused flags, fee config, XLM token address,
//!     accumulated fees, and the progress-contract link.
//!
//! The fee config and XLM token address are checked both directly
//! (`get_fee_config`, `get_accumulated_fees`) and behaviourally — a fresh
//! subscription after the upgrade only succeeds if the surviving instance-stored
//! XLM token address and fee config are still usable.

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Bytes, Env,
};

fn default_fees() -> FeeConfig {
    // NOTE: the current `FeeConfig` on this branch has eight fields (the two
    // `trial_offer_*` fields were added to the struct); a struct literal must
    // set them all, so this harness constructs the full record even though the
    // repo's older `default_fees()` test helpers only set six.
    FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: 1_000_000,
        trial_offer_expiry_secs: 7 * 24 * 60 * 60,
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    env: Env,
    xlm: Address,
    scout_access: ScoutAccessContractClient<'static>,
}

struct Seeded {
    scout: Address,
    player_id: u64,
}

fn rehearse_upgrade(h: &Harness) {
    let new_wasm_hash = h.env.deployer().upload_contract_wasm(Bytes::new(&h.env));
    h.scout_access.upgrade(&new_wasm_hash);
}

fn fund(h: &Harness, who: &Address, amount: i128) {
    StellarAssetClient::new(&h.env, &h.xlm).mint(who, &amount);
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
    let scout_access = ScoutAccessContractClient::new(&env, &id);
    scout_access.initialize(&admin, &xlm, &default_fees());

    Harness {
        env,
        xlm,
        scout_access,
    }
}

/// Seed an Elite subscription, one contact record, and the progress-contract
/// link.
fn seed(h: &Harness) -> Seeded {
    let scout = Address::generate(&h.env);
    let player_id: u64 = 1;

    fund(h, &scout, 50_000_000);
    h.scout_access.subscribe(&scout, &SubscriptionTier::Elite);
    h.scout_access.pay_to_contact(&scout, &player_id);

    let progress_link = Address::generate(&h.env);
    h.scout_access.set_progress_contract(&progress_link);

    Seeded { scout, player_id }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full rehearsal for `scout_access`.
#[test]
fn test_scout_access_upgrade_preserves_state() {
    let h = setup();
    let s = seed(&h);

    // --- Snapshot (pre-upgrade) ---
    let sub_before = h.scout_access.get_subscription(&s.scout);
    let contacted = h.scout_access.has_contacted(&s.scout, &s.player_id);
    let fees_before = h.scout_access.get_fee_config();
    let accumulated = h.scout_access.get_accumulated_fees(); // instance counter
    let health = h.scout_access.health();

    assert_eq!(sub_before.tier, SubscriptionTier::Elite);
    assert!(contacted);
    assert_eq!(accumulated, 7_000_000 + 100_000); // elite sub + contact fee
    assert!(health.initialized && !health.paused);

    // --- Upgrade ---
    rehearse_upgrade(&h);

    // Re-wire the progress-contract link (plain overwrite for scout_access).
    let new_progress_link = Address::generate(&h.env);
    h.scout_access.set_progress_contract(&new_progress_link);

    // --- Assert: persistent storage survived (subscription + contact records) ---
    let sub_after = h.scout_access.get_subscription(&s.scout);
    assert_eq!(sub_after.scout, sub_before.scout);
    assert_eq!(sub_after.tier, SubscriptionTier::Elite);
    assert_eq!(sub_after.expires_at, sub_before.expires_at);
    assert_eq!(sub_after.subscribed_at, sub_before.subscribed_at);
    assert!(h.scout_access.has_contacted(&s.scout, &s.player_id));

    // --- Assert: instance state survived (fee config, XLM token, counters, flags) ---
    let fees_after = h.scout_access.get_fee_config();
    assert_eq!(
        fees_after.contact_fee_stroops,
        fees_before.contact_fee_stroops
    );
    assert_eq!(fees_after.elite_sub_stroops, fees_before.elite_sub_stroops);
    assert_eq!(fees_after.sub_duration_secs, fees_before.sub_duration_secs);
    assert_eq!(fees_after.pro_contact_limit, fees_before.pro_contact_limit);
    assert_eq!(h.scout_access.get_accumulated_fees(), accumulated);
    assert_eq!(h.scout_access.health(), health);

    // --- Assert: XLM token + fee config are still usable behaviourally ---
    // A brand-new Basic subscription only works if the instance-stored XLM token
    // address and fee config both survived the swap.
    let scout2 = Address::generate(&h.env);
    fund(&h, &scout2, 50_000_000);
    h.scout_access.subscribe(&scout2, &SubscriptionTier::Basic);
    assert_eq!(
        h.scout_access.get_subscription(&scout2).tier,
        SubscriptionTier::Basic
    );

    // --- Assert: Admin (persistent) survived — admin-gated call still works ---
    h.scout_access.pause_contract();
    assert!(h.scout_access.health().paused);
    h.scout_access.unpause_contract();
    assert!(!h.scout_access.health().paused);
}

/// Deliberately-broken upgrade — proves the harness is not a no-op.
///
/// The operator forgets to re-verify the instance `Paused` flag after the
/// upgrade and the contract is left paused. The harness's post-upgrade
/// functional check — a state-changing `subscribe` call — then panics with
/// `ContractPaused`, catching the skipped re-verification step.
#[test]
#[should_panic]
fn test_scout_access_broken_upgrade_left_paused_is_caught() {
    let h = setup();
    let _ = seed(&h);

    rehearse_upgrade(&h);

    // Simulate a botched re-verification: the contract is (still) paused.
    h.scout_access.pause_contract();

    // Post-upgrade functional check — must not silently succeed while paused.
    let scout2 = Address::generate(&h.env);
    fund(&h, &scout2, 50_000_000);
    h.scout_access.subscribe(&scout2, &SubscriptionTier::Basic);
}
