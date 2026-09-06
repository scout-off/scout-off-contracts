//! Tests for the `EvidenceAccessGrant` confidential-evidence authorization
//! system — issue #1040, specified in `docs/EVIDENCE_PRIVACY.md`.
//!
//! Covers: grant issuance is atomic with a successful `pay_to_contact` /
//! `batch_contact_players` call, idempotent against the existing
//! "already contacted" guard, never issued on a rejected call, the query
//! API (`has_evidence_access` / `get_evidence_access_grant` /
//! `get_player_access_grants`), the admin-revoke path, and the
//! append-only-grant-vs-live-entitlement lifecycle decision (a grant
//! survives subscription downgrade/expiry).
//!
//! Self-contained: only depends on `scoutchain_scout_access` + `soroban_sdk`
//! (no cross-contract harness), matching the style of
//! `check_precedence_property_tests.rs`.

use scoutchain_scout_access::{
    EvidenceAccessGrant, FeeConfig, ScoutAccessContract, ScoutAccessContractClient,
    SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, IntoVal, Symbol,
};

const CONTACT_FEE: i128 = 100_000;
const BASIC_FEE: i128 = 1_000_000;
const PRO_FEE: i128 = 3_000_000;
const ELITE_FEE: i128 = 7_000_000;
const SUB_DURATION: u64 = 30 * 24 * 3600;
const START_TIME: u64 = 10_000_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: BASIC_FEE,
        pro_sub_stroops: PRO_FEE,
        elite_sub_stroops: ELITE_FEE,
        sub_duration_secs: SUB_DURATION,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
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
    env.ledger().with_mut(|l| l.timestamp = START_TIME);

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

fn fund(h: &Harness, addr: &Address, amount: i128) {
    StellarAssetClient::new(&h.env, &h.xlm).mint(addr, &amount);
}

fn subscribe(h: &Harness, scout: &Address, tier: &SubscriptionTier) {
    let fee = match tier {
        SubscriptionTier::Basic => BASIC_FEE,
        SubscriptionTier::Pro => PRO_FEE,
        SubscriptionTier::Elite => ELITE_FEE,
    };
    fund(h, scout, fee + CONTACT_FEE * 100);
    h.contract.subscribe(scout, tier);
}

// ---------------------------------------------------------------------------
// Grant issuance is atomic with a successful pay_to_contact
// ---------------------------------------------------------------------------

#[test]
fn pay_to_contact_success_grants_evidence_access() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);

    assert!(!h.contract.has_evidence_access(&player_id, &scout));

    h.contract.pay_to_contact(&scout, &player_id);

    assert!(h.contract.has_evidence_access(&player_id, &scout));
    let grant = h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .expect("grant must exist after a successful pay_to_contact");
    assert_eq!(grant.player_id, player_id);
    assert_eq!(grant.scout, scout);
    assert_eq!(grant.granted_at, START_TIME);
    assert_eq!(grant.tier_at_grant, SubscriptionTier::Elite);
    assert!(!grant.revoked);
    assert_eq!(grant.revoked_at, None);
}

#[test]
fn pay_to_contact_emits_evidence_access_granted_event() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Pro);

    h.contract.pay_to_contact(&scout, &player_id);

    // evidence_access_granted is published before player_contacted (see
    // pay_to_contact in lib.rs).
    assert_eq!(
        h.env.events().all().filter_by_contract(&h.contract.address),
        vec![
            &h.env,
            (
                h.contract.address.clone(),
                (
                    Symbol::new(&h.env, "evidence_access_granted"),
                    scout.clone(),
                )
                    .into_val(&h.env),
                (player_id, SubscriptionTier::Pro).into_val(&h.env),
            ),
            (
                h.contract.address.clone(),
                (Symbol::new(&h.env, "player_contacted"), scout.clone()).into_val(&h.env),
                (player_id, CONTACT_FEE).into_val(&h.env),
            ),
        ]
    );
}

#[test]
fn batch_contact_players_grants_evidence_access_for_each_new_contact() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout, &SubscriptionTier::Elite);

    let player_ids = soroban_sdk::vec![&h.env, 1u64, 2u64, 3u64];
    h.contract.batch_contact_players(&scout, &player_ids);

    for player_id in [1u64, 2u64, 3u64] {
        assert!(
            h.contract.has_evidence_access(&player_id, &scout),
            "player {player_id} must have a grant after batch_contact_players"
        );
    }
}

#[test]
fn batch_contact_players_does_not_duplicate_grant_for_already_contacted_player() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout, &SubscriptionTier::Elite);

    h.contract.pay_to_contact(&scout, &1u64);
    let first_grant = h.contract.get_evidence_access_grant(&1u64, &scout).unwrap();

    // Re-contact player 1 (already contacted) alongside a genuinely new player 2.
    let player_ids = soroban_sdk::vec![&h.env, 1u64, 2u64];
    let new_contacts = h.contract.batch_contact_players(&scout, &player_ids);
    assert_eq!(
        new_contacts, 1,
        "player 1 was already contacted, only player 2 is new"
    );

    let grants = h.contract.get_player_access_grants(&1u64, &0u32, &10u32);
    assert_eq!(
        grants.len(),
        1,
        "no duplicate grant for the already-contacted player"
    );
    assert_eq!(grants.get(0).unwrap().granted_at, first_grant.granted_at);
}

// ---------------------------------------------------------------------------
// Idempotency: a rejected pay_to_contact never grants access
// ---------------------------------------------------------------------------

#[test]
fn pay_to_contact_already_contacted_does_not_duplicate_grant_or_event() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);

    h.contract.pay_to_contact(&scout, &player_id);
    let first_grant = h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .unwrap();

    let result = h.contract.try_pay_to_contact(&scout, &player_id);
    assert!(
        result.is_err(),
        "second contact must be rejected as AlreadyContacted"
    );

    let grants = h
        .contract
        .get_player_access_grants(&player_id, &0u32, &10u32);
    assert_eq!(
        grants.len(),
        1,
        "AlreadyContacted must not issue a second grant"
    );
    assert_eq!(
        grants.get(0).unwrap().granted_at,
        first_grant.granted_at,
        "the surviving grant must be the original one, unmodified"
    );
}

/// Property test: no code path that returns an error from `pay_to_contact`
/// may have written an `EvidenceAccessGrant`.
#[test]
fn rejected_pay_to_contact_never_grants_access_not_subscribed() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    fund(&h, &scout, CONTACT_FEE * 10);

    let result = h.contract.try_pay_to_contact(&scout, &player_id);
    assert!(result.is_err());
    assert!(!h.contract.has_evidence_access(&player_id, &scout));
    assert!(h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .is_none());
}

#[test]
fn rejected_pay_to_contact_never_grants_access_expired_subscription() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.env.ledger().with_mut(|l| l.timestamp += SUB_DURATION + 1);

    let result = h.contract.try_pay_to_contact(&scout, &player_id);
    assert!(result.is_err());
    assert!(!h.contract.has_evidence_access(&player_id, &scout));
}

#[test]
fn rejected_pay_to_contact_never_grants_access_pro_quota_exceeded() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout, &SubscriptionTier::Pro);

    // Exhaust the Pro-tier quota (10/period).
    for player_id in 1u64..=10u64 {
        h.contract.pay_to_contact(&scout, &player_id);
    }
    let over_quota_player = 999u64;
    let result = h.contract.try_pay_to_contact(&scout, &over_quota_player);
    assert!(result.is_err());
    assert!(!h.contract.has_evidence_access(&over_quota_player, &scout));
}

#[test]
#[should_panic]
fn rejected_pay_to_contact_never_grants_access_insufficient_balance() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;

    // Subscribe with exactly the subscription fee — nothing left for the
    // contact fee, so the token transfer inside pay_to_contact panics.
    fund(&h, &scout, ELITE_FEE);
    h.contract.subscribe(&scout, &SubscriptionTier::Elite);

    h.contract.pay_to_contact(&scout, &player_id);
    // Unreachable, but documents the intended assertion if the panic
    // behavior above ever changes to a graceful Result::Err instead.
    assert!(!h.contract.has_evidence_access(&player_id, &scout));
}

#[test]
fn rejected_pay_to_contact_never_grants_access_when_paused() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pause_contract();

    let result = h.contract.try_pay_to_contact(&scout, &player_id);
    assert!(result.is_err());
    assert!(!h.contract.has_evidence_access(&player_id, &scout));
}

// ---------------------------------------------------------------------------
// Lifecycle: grants are append-only facts, not live entitlements
// ---------------------------------------------------------------------------

#[test]
fn grant_survives_subscription_expiry() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pay_to_contact(&scout, &player_id);
    assert!(h.contract.has_evidence_access(&player_id, &scout));

    // Let the subscription lapse.
    h.env.ledger().with_mut(|l| l.timestamp += SUB_DURATION + 1);

    // The grant this scout already earned is not revoked by expiry.
    assert!(h.contract.has_evidence_access(&player_id, &scout));
    let grant = h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .unwrap();
    assert!(!grant.revoked);
}

#[test]
fn grant_survives_subscription_downgrade() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pay_to_contact(&scout, &player_id);
    assert!(h.contract.has_evidence_access(&player_id, &scout));

    // Let the Elite subscription lapse, then re-subscribe at Basic — a
    // scout can only move to a cheaper tier once the prior one has expired
    // (SubscriptionDowngradeNotAllowed guards a live downgrade), but the
    // net effect on evidence access is the same lifecycle question: does a
    // scout who is no longer entitled to *new* contacts keep the grant they
    // already earned? Yes.
    h.env.ledger().with_mut(|l| l.timestamp += SUB_DURATION + 1);
    subscribe(&h, &scout, &SubscriptionTier::Basic);

    assert!(h.contract.has_evidence_access(&player_id, &scout));
}

// ---------------------------------------------------------------------------
// Query API: get_player_access_grants pagination
// ---------------------------------------------------------------------------

#[test]
fn get_player_access_grants_pagination() {
    let h = setup();
    let player_id = 1u64;
    let mut scouts = Vec::new();
    for _ in 0..5 {
        let scout = Address::generate(&h.env);
        subscribe(&h, &scout, &SubscriptionTier::Elite);
        h.contract.pay_to_contact(&scout, &player_id);
        scouts.push(scout);
    }

    let page1 = h
        .contract
        .get_player_access_grants(&player_id, &0u32, &2u32);
    assert_eq!(page1.len(), 2);
    let page2 = h
        .contract
        .get_player_access_grants(&player_id, &2u32, &2u32);
    assert_eq!(page2.len(), 2);
    let page3 = h
        .contract
        .get_player_access_grants(&player_id, &4u32, &2u32);
    assert_eq!(page3.len(), 1);
    let page4 = h
        .contract
        .get_player_access_grants(&player_id, &5u32, &2u32);
    assert_eq!(page4.len(), 0);

    // Grants come back oldest-first, in issuance order.
    for (i, scout) in scouts.iter().enumerate() {
        let expected = h
            .contract
            .get_evidence_access_grant(&player_id, scout)
            .unwrap();
        let got = if i < 2 {
            page1.get(i as u32).unwrap()
        } else if i < 4 {
            page2.get((i - 2) as u32).unwrap()
        } else {
            page3.get((i - 4) as u32).unwrap()
        };
        assert_eq!(got.scout, expected.scout);
    }
}

#[test]
fn get_player_access_grants_empty_for_unknown_player() {
    let h = setup();
    let grants = h
        .contract
        .get_player_access_grants(&12345u64, &0u32, &10u32);
    assert_eq!(grants.len(), 0);
}

// ---------------------------------------------------------------------------
// admin_revoke_evidence_access
// ---------------------------------------------------------------------------

#[test]
fn admin_revoke_evidence_access_flips_revoked_and_keeps_record() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pay_to_contact(&scout, &player_id);
    assert!(h.contract.has_evidence_access(&player_id, &scout));

    h.contract.admin_revoke_evidence_access(&player_id, &scout);

    assert!(
        !h.contract.has_evidence_access(&player_id, &scout),
        "revoked grant must no longer read as active access"
    );
    let grant = h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .expect("revoke must not delete the grant record — it's an append-only fact");
    assert!(grant.revoked);
    assert_eq!(grant.revoked_at, Some(h.env.ledger().timestamp()));

    // Still enumerable — a revoked grant remains part of the audit trail.
    let grants = h
        .contract
        .get_player_access_grants(&player_id, &0u32, &10u32);
    assert_eq!(grants.len(), 1);
    assert!(grants.get(0).unwrap().revoked);
}

#[test]
fn admin_revoke_evidence_access_emits_event() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pay_to_contact(&scout, &player_id);

    h.contract.admin_revoke_evidence_access(&player_id, &scout);

    assert_eq!(
        h.env.events().all().filter_by_contract(&h.contract.address),
        vec![
            &h.env,
            (
                h.contract.address.clone(),
                (
                    Symbol::new(&h.env, "evidence_access_revoked"),
                    scout.clone(),
                )
                    .into_val(&h.env),
                (player_id, h.admin.clone()).into_val(&h.env),
            ),
        ]
    );
}

#[test]
fn admin_revoke_evidence_access_grant_not_found() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let result = h.contract.try_admin_revoke_evidence_access(&1u64, &scout);
    let err = result
        .expect_err("revoking a grant that was never issued must fail")
        .expect("must be a contract error, not a host error");
    assert_eq!(
        err,
        scoutchain_scout_access::ScoutAccessError::GrantNotFound
    );
}

#[test]
fn admin_revoke_evidence_access_is_idempotent() {
    let h = setup();
    let scout = Address::generate(&h.env);
    let player_id = 1u64;
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pay_to_contact(&scout, &player_id);

    h.contract.admin_revoke_evidence_access(&player_id, &scout);
    let first_revoked_at = h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .unwrap()
        .revoked_at;

    h.env.ledger().with_mut(|l| l.timestamp += 1);
    let result = h
        .contract
        .try_admin_revoke_evidence_access(&player_id, &scout);
    assert!(
        result.is_ok(),
        "revoking an already-revoked grant must be a graceful no-op"
    );

    let grant = h
        .contract
        .get_evidence_access_grant(&player_id, &scout)
        .unwrap();
    assert_eq!(
        grant.revoked_at, first_revoked_at,
        "re-revoking must not overwrite the original revocation timestamp"
    );
}

#[test]
fn admin_revoke_evidence_access_requires_initialized_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let scout = Address::generate(&env);
    let id = env.register(ScoutAccessContract, ());
    let contract = ScoutAccessContractClient::new(&env, &id);
    let _ = admin;

    let result = contract.try_admin_revoke_evidence_access(&1u64, &scout);
    let err = result.expect_err("uninitialized contract has no admin to authorize the call");
    assert!(
        err.is_err() || err.is_ok(),
        "either representation is fine — just must not succeed"
    );
}

/// Type import sanity: `EvidenceAccessGrant` is exported from the crate root
/// so off-chain/bindings code can name it directly.
#[test]
fn evidence_access_grant_type_is_exported() {
    fn accepts(_: EvidenceAccessGrant) {}
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe(&h, &scout, &SubscriptionTier::Elite);
    h.contract.pay_to_contact(&scout, &1u64);
    let grant = h.contract.get_evidence_access_grant(&1u64, &scout).unwrap();
    accepts(grant);
}
