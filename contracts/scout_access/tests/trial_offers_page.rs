//! Tests for `get_trial_offers_page` (issue #1636).

use scoutchain_scout_access::{
    DataKey, FeeConfig, ScoutAccessContract, ScoutAccessContractClient, TrialOffer,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

const PLAYER: u64 = 7;

fn setup(n: u32) -> (Env, ScoutAccessContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &id);
    client.initialize(
        &admin,
        &xlm,
        &FeeConfig {
            contact_fee_stroops: 100_000,
            basic_sub_stroops: 1_000_000,
            pro_sub_stroops: 3_000_000,
            elite_sub_stroops: 7_000_000,
            sub_duration_secs: 30 * 24 * 60 * 60,
            pro_contact_limit: 50,
            trial_offer_escrow_stroops: 500_000,
            trial_offer_expiry_secs: 3_600,
        },
    );
    let scout = Address::generate(&env);
    env.as_contract(&id, || {
        for i in 1..=n {
            let offer = TrialOffer {
                player_id: PLAYER,
                scout: scout.clone(),
                details_hash: String::from_str(
                    &env,
                    "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB",
                ),
                logged_at: i as u64,
            };
            env.storage()
                .persistent()
                .set(&DataKey::TrialOffer(PLAYER, i), &offer);
        }
        env.storage()
            .persistent()
            .set(&DataKey::TrialCounter(PLAYER), &n);
    });
    (env, client)
}

#[test]
fn empty_player() {
    let (_e, c) = setup(0);
    let p = c.get_trial_offers_page(&PLAYER, &0, &10);
    assert_eq!(p.total, 0);
    assert_eq!(p.entries.len(), 0);
}

#[test]
fn single_offer() {
    let (_e, c) = setup(1);
    let p = c.get_trial_offers_page(&PLAYER, &0, &10);
    assert_eq!(p.total, 1);
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.entries.get(0).unwrap().index, 1);
}

#[test]
fn twenty_and_twenty_one_offers_are_not_truncated() {
    let (_e, c) = setup(20);
    assert_eq!(c.get_trial_offers_page(&PLAYER, &0, &50).entries.len(), 20);
    let (_e, c) = setup(21);
    let p = c.get_trial_offers_page(&PLAYER, &0, &50);
    assert_eq!(p.total, 21);
    assert_eq!(p.entries.len(), 21);
    // Newest first, so the 21st offer (missed by get_all_trial_offers) leads.
    let first = p.entries.get(0).unwrap();
    assert_eq!(first.index, 21);
    assert_eq!(first.offer.logged_at, 21);
    assert_eq!(p.entries.get(20).unwrap().index, 1);
    assert_eq!(c.get_all_trial_offers(&PLAYER).len(), 20);
}

#[test]
fn seventy_five_offers_walk_all_pages() {
    let (_e, c) = setup(75);
    let mut seen = 0u32;
    let mut expect = 75u32;
    let mut offset = 0u32;
    loop {
        let p = c.get_trial_offers_page(&PLAYER, &offset, &50);
        assert_eq!(p.total, 75);
        if p.entries.is_empty() {
            break;
        }
        for e in p.entries.iter() {
            assert_eq!(e.index, expect);
            expect -= 1;
            seen += 1;
        }
        offset += p.entries.len();
    }
    assert_eq!(seen, 75);
}

#[test]
fn offset_and_limit_boundaries() {
    let (_e, c) = setup(75);
    // limit clamped to 50
    assert_eq!(c.get_trial_offers_page(&PLAYER, &0, &200).entries.len(), 50);
    // limit 0 treated as 1
    assert_eq!(c.get_trial_offers_page(&PLAYER, &0, &0).entries.len(), 1);
    // last item
    let p = c.get_trial_offers_page(&PLAYER, &74, &10);
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.entries.get(0).unwrap().index, 1);
    // offset == total and beyond: empty page, total still reported
    for off in [75u32, 1000] {
        let p = c.get_trial_offers_page(&PLAYER, &off, &10);
        assert_eq!(p.entries.len(), 0);
        assert_eq!(p.total, 75);
    }
}
