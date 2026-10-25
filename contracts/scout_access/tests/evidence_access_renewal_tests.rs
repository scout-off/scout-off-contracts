//! Tests for issue #1607: renewing an expired evidence-access grant
//! (`renew_evidence_access`) and player re-approval of a revoked grant
//! (`reinstate_evidence_access`).

mod common;

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, ScoutAccessError, SubscriptionTier,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

const CONTACT_FEE: i128 = 100_000;
const SUB_DURATION: u64 = 200 * 24 * 3600;
const START_TIME: u64 = 10_000_000;
const GRANT_TTL: u64 = 90 * 24 * 3600;

struct Harness {
    env: Env,
    contract: ScoutAccessContractClient<'static>,
    scout: Address,
    player: Address,
}

const PLAYER_ID: u64 = 1;

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START_TIME);

    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let id = env.register(
        ScoutAccessContract,
        (
            admin.clone(),
            xlm.clone(),
            FeeConfig {
            contact_fee_stroops: CONTACT_FEE,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: SUB_DURATION,
            pro_contact_limit: 10,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        },
        ),
    );
    let contract = ScoutAccessContractClient::new(&env, &id);
    let reg = common::wire_registration(&env, Some(&contract), None);
    common::wire_progress_level_stub(&env, &contract);

    let player = Address::generate(&env);
    reg.map_player(&player, &PLAYER_ID);

    let scout = Address::generate(&env);
    StellarAssetClient::new(&env, &xlm).mint(&scout, &100_000_000);
    contract.subscribe(&scout, &SubscriptionTier::Elite);
    contract.pay_to_contact(&scout, &PLAYER_ID);

    Harness {
        env,
        contract,
        scout,
        player,
    }
}

fn advance(h: &Harness, secs: u64) {
    h.env.ledger().with_mut(|l| l.timestamp += secs);
}

#[test]
fn expired_grant_can_be_renewed_by_paying() {
    let h = setup();
    advance(&h, GRANT_TTL + 1);
    assert!(!h.contract.has_evidence_access(&PLAYER_ID, &h.scout));
    let fees_before = h.contract.get_accumulated_fees();

    h.contract.renew_evidence_access(&h.scout, &PLAYER_ID);

    assert!(h.contract.has_evidence_access(&PLAYER_ID, &h.scout));
    let grant = h
        .contract
        .get_evidence_access_grant(&PLAYER_ID, &h.scout)
        .unwrap();
    assert_eq!(grant.expires_at, START_TIME + GRANT_TTL + 1 + GRANT_TTL);
    assert_eq!(h.contract.get_accumulated_fees(), fees_before + CONTACT_FEE);
}

#[test]
fn renewal_does_not_duplicate_enumeration_entry() {
    let h = setup();
    advance(&h, GRANT_TTL + 1);
    h.contract.renew_evidence_access(&h.scout, &PLAYER_ID);
    let grants = h.contract.get_player_access_grants(&PLAYER_ID, &0, &50);
    assert_eq!(grants.len(), 1);
}

#[test]
fn active_grant_cannot_be_renewed() {
    let h = setup();
    let r = h.contract.try_renew_evidence_access(&h.scout, &PLAYER_ID);
    assert_eq!(r, Err(Ok(ScoutAccessError::GrantNotExpired)));
}

#[test]
fn renewal_without_contact_is_rejected() {
    let h = setup();
    let r = h.contract.try_renew_evidence_access(&h.scout, &2u64);
    assert_eq!(r, Err(Ok(ScoutAccessError::ContactNotFound)));
}

#[test]
fn revoked_grant_cannot_be_renewed_but_can_be_reinstated_by_player() {
    let h = setup();
    h.contract
        .revoke_evidence_access(&h.player, &PLAYER_ID, &h.scout);
    assert!(!h.contract.has_evidence_access(&PLAYER_ID, &h.scout));
    advance(&h, GRANT_TTL + 1);
    let r = h.contract.try_renew_evidence_access(&h.scout, &PLAYER_ID);
    assert_eq!(r, Err(Ok(ScoutAccessError::GrantAlreadyRevoked)));

    let fees_before = h.contract.get_accumulated_fees();
    h.contract
        .reinstate_evidence_access(&h.player, &PLAYER_ID, &h.scout);

    assert!(h.contract.has_evidence_access(&PLAYER_ID, &h.scout));
    let grant = h
        .contract
        .get_evidence_access_grant(&PLAYER_ID, &h.scout)
        .unwrap();
    assert!(!grant.revoked);
    assert_eq!(grant.revoked_at, None);
    assert_eq!(h.contract.get_accumulated_fees(), fees_before);
    assert_eq!(
        h.contract
            .get_player_access_grants(&PLAYER_ID, &0, &50)
            .len(),
        1
    );
}

#[test]
fn reinstate_requires_revoked_grant_and_player_ownership() {
    let h = setup();
    let r = h
        .contract
        .try_reinstate_evidence_access(&h.player, &PLAYER_ID, &h.scout);
    assert_eq!(r, Err(Ok(ScoutAccessError::GrantNotRevoked)));

    h.contract
        .revoke_evidence_access(&h.player, &PLAYER_ID, &h.scout);
    let stranger = Address::generate(&h.env);
    let r = h
        .contract
        .try_reinstate_evidence_access(&stranger, &PLAYER_ID, &h.scout);
    assert_eq!(r, Err(Ok(ScoutAccessError::PlayerNotVerified)));
}

#[test]
fn player_cannot_reinstate_admin_revoked_grant() {
    let h = setup();
    h.contract
        .admin_revoke_evidence_access(&PLAYER_ID, &h.scout);
    let r = h
        .contract
        .try_reinstate_evidence_access(&h.player, &PLAYER_ID, &h.scout);
    assert_eq!(r, Err(Ok(ScoutAccessError::GrantAdminRevoked)));
    assert!(!h.contract.has_evidence_access(&PLAYER_ID, &h.scout));
}
