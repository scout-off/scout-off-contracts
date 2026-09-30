//! Tests for issue #817: unified Pro-quota error codes.
//!
//! Verifies that both `pay_to_contact` and `batch_contact_players` return
//! the same error code (`ProContactLimitReached` = 20) when the Pro-tier
//! monthly contact limit is exceeded.

use scoutchain_scout_access::{FeeConfig, ScoutAccessContractClient, SubscriptionTier};
use scoutchain_verification::VerificationContractClient;
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

const CONTACT_FEE: i128 = 100_000;
const PRO_SUB: i128 = 3_000_000;

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: CONTACT_FEE,
        basic_sub_stroops: PRO_SUB,
        pro_sub_stroops: PRO_SUB,
        elite_sub_stroops: PRO_SUB,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 2,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 7_200,
    }
}

fn setup() -> (
    Env,
    ScoutAccessContractClient<'static>,
    VerificationContractClient<'static>,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let verification_id = env.register(scoutchain_verification::VerificationContract, ());
    let verification_client =
        scoutchain_verification::VerificationContractClient::new(&env, &verification_id);

    let scout_access_id = env.register(scoutchain_scout_access::ScoutAccessContract, ());
    let scout_access_client =
        scoutchain_scout_access::ScoutAccessContractClient::new(&env, &scout_access_id);

    let admin = Address::generate(&env);
    let scout = Address::generate(&env);
    let player = Address::generate(&env);

    let xlm_token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    StellarAssetClient::new(&env, &xlm_token).mint(&scout, &1_000_000_000i128);
    let fee_config = default_fees();

    scout_access_client.initialize(&admin, &xlm_token, &fee_config);
    verification_client.initialize(&admin);

    (env, scout_access_client, verification_client, scout, player)
}

#[test]
fn test_pay_to_contact_and_batch_contact_players_return_same_quota_error() {
    let (env, scout_access, _verification, scout, _player) = setup();

    scout_access.subscribe(&scout, &SubscriptionTier::Pro);

    // pro_contact_limit is 2: exactly two distinct contacts are allowed.

    let result_pay = scout_access.try_pay_to_contact(&scout, &1u64);
    assert!(result_pay.is_ok(), "First contact should succeed");

    // Second contact via batch — a *distinct* player (2), so it is not a no-op.
    let player_ids = soroban_sdk::Vec::from_slice(&env, &[2u64]);
    let result_batch = scout_access.try_batch_contact_players(&scout, &player_ids);
    assert!(
        result_batch.is_ok(),
        "Second contact via batch should succeed"
    );

    let result_pay_third = scout_access.try_pay_to_contact(&scout, &3u64);
    let expected_error = scoutchain_scout_access::ScoutAccessError::ProContactLimitReached;
    assert_eq!(
        result_pay_third,
        Err(Ok(expected_error)),
        "pay_to_contact should return ProContactLimitReached (20)"
    );

    let player_ids_exceed = soroban_sdk::Vec::from_slice(&env, &[4u64]);
    let result_batch_exceed = scout_access.try_batch_contact_players(&scout, &player_ids_exceed);
    assert_eq!(
        result_batch_exceed,
        Err(Ok(expected_error)),
        "batch_contact_players should return ProContactLimitReached (20)"
    );
}

#[test]
fn test_batch_contact_players_pro_quota_exceeded_returns_pro_contact_limit_reached() {
    let (env, scout_access, _verification, scout, _player) = setup();

    scout_access.subscribe(&scout, &SubscriptionTier::Pro);

    let player_ids = soroban_sdk::Vec::from_slice(&env, &[1u64, 2u64, 3u64]);

    let result = scout_access.try_batch_contact_players(&scout, &player_ids);
    let expected_error = scoutchain_scout_access::ScoutAccessError::ProContactLimitReached;
    assert_eq!(
        result,
        Err(Ok(expected_error)),
        "batch_contact_players should return ProContactLimitReached (20) when Pro limit exceeded"
    );
}

#[test]
fn test_contact_quota_exceeded_code_18_is_deprecated() {
    use scoutchain_scout_access::ScoutAccessError;
    assert_eq!(ScoutAccessError::ContactQuotaExceeded as u32, 18);
    assert_eq!(ScoutAccessError::ProContactLimitReached as u32, 20);
    assert_ne!(
        ScoutAccessError::ContactQuotaExceeded as u32,
        ScoutAccessError::ProContactLimitReached as u32,
        "Codes 18 and 20 must remain distinct even though 18 is deprecated"
    );
}
