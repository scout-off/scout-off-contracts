//! Basic behaviour of `scout_access.purge_player_data` (issue #1891).
//!
//! Pins the current contract for a player with fewer than one page (50) of
//! trial offers: refusal while an unexpired escrow exists, removal of the
//! per-player keys once escrows are refunded, admin-only access, pause
//! behaviour and the `PlayerDataPurged` event payload.

mod common;

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_scout_access::{
    events::PlayerDataPurged, DataKey, FeeConfig, ScoutAccessContract, ScoutAccessContractClient,
    ScoutAccessError, SubscriptionTier,
};
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
    token::StellarAssetClient,
    Address, Env, IntoVal, String, TryFromVal,
};

const ESCROW_AMOUNT: i128 = 500_000;
const EXPIRY_SECS: u64 = 3_600;
const START_TS: u64 = 10_000_000;
const PLAYER_ID: u64 = 1;

const CID_1: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const CID_2: &str = "QmwsjoZwgfzgx6xPr3cXEKhzfLt5RQ87yMnWecTp1tf6p7";
const CID_3: &str = "QmgzsER5ykyxoTsVUSePRkKXqkEzsRVLpUv511dp4c3vAs";
const CID_4: &str = "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG";

struct Harness {
    env: Env,
    admin: Address,
    xlm: Address,
    progress: ProgressContractClient<'static>,
    scout_access: ScoutAccessContractClient<'static>,
    verification: VerificationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START_TS);

    let admin = Address::generate(&env);

    let ver_id = env.register(VerificationContract, (admin.clone(),));
    let verification = VerificationContractClient::new(&env, &ver_id);
    verification.set_registration_contract(
        &scoutchain_shared_types::testutils::active_registry_stub(&env),
    );

    let progress_id = env.register(ProgressContract, (admin.clone(),));
    let progress = ProgressContractClient::new(&env, &progress_id);
    common::wire_registration(&env, None, Some(&progress));
    progress.set_verification_contract(&ver_id);

    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let fees = FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: ESCROW_AMOUNT,
        trial_offer_expiry_secs: EXPIRY_SECS,
    };
    let sa_id = env.register(ScoutAccessContract, (admin.clone(), xlm.clone(), fees));
    let scout_access = ScoutAccessContractClient::new(&env, &sa_id);
    common::wire_registration(&env, Some(&scout_access), None);

    scout_access.set_progress_contract(&progress_id);
    progress.set_scout_access_contract(&sa_id);

    // Player must be at PerformanceMilestones (level 2) to receive trial offers.
    for i in 1..=2u32 {
        progress.advance_level(&verification.address, &PLAYER_ID, &i);
    }

    Harness {
        env,
        admin,
        xlm,
        progress,
        scout_access,
        verification,
    }
}

fn approve_milestone(h: &Harness, evidence_hash: &str) {
    let validator = Address::generate(&h.env);
    h.verification.register_validator(
        &validator,
        &String::from_str(&h.env, "UEFA-B-License"),
        &String::from_str(&h.env, "Default Academy"),
        &soroban_sdk::Vec::new(&h.env),
    );
    h.verification.approve_milestone(
        &validator,
        &PLAYER_ID,
        &String::from_str(&h.env, "scored"),
        &String::from_str(&h.env, evidence_hash),
        &None,
    );
}

/// Two Elite scouts each pay-to-contact the player and log one trial offer
/// (a single scout is limited to one offer per player per 24h).
fn contact_and_log_two_offers(h: &Harness) {
    for (evidence, details) in [(CID_1, CID_2), (CID_3, CID_4)] {
        let scout = Address::generate(&h.env);
        StellarAssetClient::new(&h.env, &h.xlm).mint(&scout, &20_000_000i128);
        h.scout_access.subscribe(&scout, &SubscriptionTier::Elite);
        h.scout_access.pay_to_contact(&scout, &PLAYER_ID);

        approve_milestone(h, evidence);
        h.scout_access
            .log_trial_offer(&scout, &PLAYER_ID, &String::from_str(&h.env, details));
    }
}

/// Let every escrow expire and refund them via the admin sweep.
fn expire_and_refund(h: &Harness) {
    h.env.ledger().with_mut(|l| l.timestamp += EXPIRY_SECS + 1);
    assert_eq!(h.scout_access.expire_trial_offers(&10u32), 2);
}

fn has_key(h: &Harness, key: &DataKey) -> bool {
    h.env.as_contract(&h.scout_access.address, || {
        h.env.storage().persistent().has(key)
    })
}

#[test]
fn purge_refused_while_unexpired_escrow_exists() {
    let h = setup();
    contact_and_log_two_offers(&h);

    let result = h.scout_access.try_purge_player_data(&PLAYER_ID, &0u32);
    assert_eq!(result, Err(Ok(ScoutAccessError::TrialOfferNotFound)));

    // Nothing was touched.
    assert_eq!(h.scout_access.get_trial_count(&PLAYER_ID), 2);
    assert_eq!(h.scout_access.get_player_contacts(&PLAYER_ID).len(), 2);
    assert!(has_key(&h, &DataKey::EvidenceAccessGrantCount(PLAYER_ID)));
}

#[test]
fn purge_succeeds_after_escrows_are_refunded() {
    let h = setup();
    contact_and_log_two_offers(&h);
    expire_and_refund(&h);

    let (next_cursor, more) = h.scout_access.purge_player_data(&PLAYER_ID, &0u32);
    assert_eq!(next_cursor, 2);
    assert!(!more);

    for idx in 1..=2u32 {
        assert!(!has_key(&h, &DataKey::TrialOffer(PLAYER_ID, idx)));
        assert!(!has_key(&h, &DataKey::TrialEscrow(PLAYER_ID, idx)));
    }
    assert!(!has_key(&h, &DataKey::TrialCounter(PLAYER_ID)));
    assert_eq!(h.scout_access.get_trial_count(&PLAYER_ID), 0);
}

#[test]
fn purge_clears_player_contacts_and_grant_count_on_first_page() {
    let h = setup();
    contact_and_log_two_offers(&h);
    assert_eq!(h.scout_access.get_player_contacts(&PLAYER_ID).len(), 2);
    assert!(has_key(&h, &DataKey::PlayerContacts(PLAYER_ID)));
    assert!(has_key(&h, &DataKey::EvidenceAccessGrantCount(PLAYER_ID)));
    expire_and_refund(&h);

    h.scout_access.purge_player_data(&PLAYER_ID, &0u32);

    assert_eq!(h.scout_access.get_player_contacts(&PLAYER_ID).len(), 0);
    assert!(!has_key(&h, &DataKey::PlayerContacts(PLAYER_ID)));
    assert!(!has_key(&h, &DataKey::EvidenceAccessGrantCount(PLAYER_ID)));
}

#[test]
fn purge_rejects_non_admin() {
    let h = setup();
    contact_and_log_two_offers(&h);
    expire_and_refund(&h);

    // Only a non-admin authorises the call: the admin's require_auth fails.
    let intruder = Address::generate(&h.env);
    h.env.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &h.scout_access.address,
            fn_name: "purge_player_data",
            args: (PLAYER_ID, 0u32).into_val(&h.env),
            sub_invokes: &[],
        },
    }]);
    let result = h.scout_access.try_purge_player_data(&PLAYER_ID, &0u32);
    assert!(result.is_err(), "non-admin purge must fail authorization");

    // State is untouched.
    assert_eq!(h.scout_access.get_trial_count(&PLAYER_ID), 2);
    assert_eq!(h.scout_access.get_player_contacts(&PLAYER_ID).len(), 2);
}

#[test]
fn purge_rejected_while_paused() {
    let h = setup();
    contact_and_log_two_offers(&h);
    expire_and_refund(&h);

    h.scout_access.pause_contract();
    let result = h.scout_access.try_purge_player_data(&PLAYER_ID, &0u32);
    assert_eq!(result, Err(Ok(ScoutAccessError::ContractPaused)));
    assert_eq!(h.scout_access.get_trial_count(&PLAYER_ID), 2);

    // Works again after unpausing.
    h.scout_access.unpause_contract();
    let (_, more) = h.scout_access.purge_player_data(&PLAYER_ID, &0u32);
    assert!(!more);
}

#[test]
fn purge_emits_event_with_entries_removed() {
    let h = setup();
    contact_and_log_two_offers(&h);
    expire_and_refund(&h);

    h.scout_access.purge_player_data(&PLAYER_ID, &0u32);
    let all = h.env.events().all();
    let events = all.events();

    let event = events
        .iter()
        .find_map(|e| match &e.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => match v0.topics.first() {
                Some(soroban_sdk::xdr::ScVal::Symbol(sym))
                    if AsRef::<[u8]>::as_ref(&sym.0) == b"player_data_purged" =>
                {
                    Some(v0)
                }
                _ => None,
            },
        })
        .expect("player_data_purged must be emitted");

    // PlayerContacts + EvidenceAccessGrantCount + TrialCounter (3) plus the two
    // TrialOffer records; both escrows were already removed by the refund sweep.
    let expected: soroban_sdk::Val = soroban_sdk::events::Event::data(
        &PlayerDataPurged {
            admin: h.admin.clone(),
            player_id: PLAYER_ID,
            entries_removed: 5,
            more: false,
        },
        &h.env,
    );
    let expected_xdr = soroban_sdk::xdr::ScVal::try_from_val(&h.env, &expected).unwrap();
    assert_eq!(event.data, expected_xdr);

    let _ = &h.progress;
}
