//! Regression tests for issue #1018: `batch_contact_players` overcharged a
//! scout when the same `player_id` appeared more than once in a single batch
//! call. The fee-counting pass didn't deduplicate the input list against
//! itself, so repeated IDs were each counted as "new" and charged for, even
//! though only one `ContactRecord` is ever actually written.
//!
//! These tests prove that the fee charged, the `ContactRecord`s written, and
//! the Pro-tier contact-count increment all match the number of *distinct*
//! player_ids in the batch, not the raw input length.

use scoutchain_scout_access::{
    FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier,
};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

const CONTACT_FEE: i128 = 100_000;
const BASIC_FEE: i128 = 1_000_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: BASIC_FEE,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 10_000_000,
        sub_duration_secs: 2_592_000,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 7_200,
    }
}

fn setup() -> (
    Env,
    ScoutAccessContractClient<'static>,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let scout_access_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &scout_access_id);

    let admin = Address::generate(&env);
    let scout = Address::generate(&env);

    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    StellarAssetClient::new(&env, &xlm).mint(&scout, &10_000_000i128);

    client.initialize(&admin, &xlm, &default_fees());
    client.subscribe(&scout, &SubscriptionTier::Basic);

    (env, client, admin, scout, xlm)
}

/// Reads the Pro/Basic-tier monthly contact-count bucket directly from
/// storage, mirroring how `increment_contact_count_by` writes it.
fn read_contact_count(env: &Env, scout: &Address, contract_id: &Address) -> u32 {
    const SECONDS_PER_MONTH: u64 = 2_592_000;
    let month_bucket = env.ledger().timestamp() / SECONDS_PER_MONTH;
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get::<scoutchain_scout_access::DataKey, u32>(
                &scoutchain_scout_access::DataKey::ContactCount(scout.clone(), month_bucket),
            )
            .unwrap_or(0)
    })
}

#[test]
fn test_batch_contact_players_dedupes_exact_duplicate_ids() {
    let (env, client, _admin, scout, xlm) = setup();

    let player_ids = soroban_sdk::vec![&env, 5u64, 5u64];
    let new_contacts = client.batch_contact_players(&scout, &player_ids);

    assert_eq!(
        new_contacts, 1,
        "only one distinct player_id in [5, 5] should be counted as new"
    );

    // Exactly one ContactRecord was written for player 5.
    assert!(client.get_contact_record(&scout, &5u64).is_some());

    // Exactly one contact_fee_stroops was charged, not two (plus the Basic
    // subscription fee paid during setup).
    let balance = soroban_sdk::token::Client::new(&env, &xlm).balance(&scout);
    assert_eq!(
        balance,
        10_000_000i128 - BASIC_FEE - CONTACT_FEE,
        "scout should only be charged for one distinct contact"
    );

    // Quota/contact-count increment reflects the deduplicated count.
    assert_eq!(
        read_contact_count(&env, &scout, &client.address),
        1,
        "contact count should increment by the deduplicated count (1), not the raw input length (2)"
    );
}

#[test]
fn test_batch_contact_players_dedupes_duplicate_among_distinct_ids() {
    let (env, client, _admin, scout, xlm) = setup();

    let player_ids = soroban_sdk::vec![&env, 5u64, 7u64, 5u64];
    let new_contacts = client.batch_contact_players(&scout, &player_ids);

    assert_eq!(
        new_contacts, 2,
        "[5, 7, 5] has 2 distinct player_ids and should count/charge for exactly 2"
    );

    assert!(client.get_contact_record(&scout, &5u64).is_some());
    assert!(client.get_contact_record(&scout, &7u64).is_some());

    let balance = soroban_sdk::token::Client::new(&env, &xlm).balance(&scout);
    assert_eq!(
        balance,
        10_000_000i128 - BASIC_FEE - (CONTACT_FEE * 2),
        "scout should be charged for exactly 2 distinct contacts, not 3"
    );

    assert_eq!(
        read_contact_count(&env, &scout, &client.address),
        2,
        "contact count should increment by the deduplicated count (2), not the raw input length (3)"
    );
}

#[test]
fn test_batch_contact_players_still_skips_already_contacted_players() {
    let (env, client, _admin, scout, xlm) = setup();

    // Pre-existing contact from a prior call.
    client.batch_contact_players(&scout, &soroban_sdk::vec![&env, 5u64]);
    let balance_after_first = soroban_sdk::token::Client::new(&env, &xlm).balance(&scout);
    assert_eq!(
        balance_after_first,
        10_000_000i128 - BASIC_FEE - CONTACT_FEE
    );

    // Second batch mixes an already-contacted id, a repeated new id, and a
    // fresh id: only the fresh id (7) should be charged for.
    let new_contacts =
        client.batch_contact_players(&scout, &soroban_sdk::vec![&env, 5u64, 7u64, 7u64]);
    assert_eq!(
        new_contacts, 1,
        "already-contacted 5 is free, and duplicate 7 counts once"
    );

    let balance_after_second = soroban_sdk::token::Client::new(&env, &xlm).balance(&scout);
    assert_eq!(
        balance_after_second,
        balance_after_first - CONTACT_FEE,
        "only the single new distinct contact (7) should be charged"
    );

    assert_eq!(
        read_contact_count(&env, &scout, &client.address),
        2,
        "total contact count across both calls should be 2 (player 5, then player 7)"
    );
}
