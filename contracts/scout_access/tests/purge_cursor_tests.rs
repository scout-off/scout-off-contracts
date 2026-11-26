use scoutchain_scout_access::{
    DataKey, FeeConfig, ScoutAccessContract, ScoutAccessContractClient, ScoutAccessError,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

const PLAYER: u64 = 7;
const TOTAL: u32 = 120;

fn setup() -> (Env, ScoutAccessContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let id = env.register(
        ScoutAccessContract,
        (
            admin,
            xlm,
            FeeConfig {
                contact_fee_stroops: 100_000,
                basic_sub_stroops: 1_000_000,
                pro_sub_stroops: 3_000_000,
                elite_sub_stroops: 7_000_000,
                sub_duration_secs: 30 * 24 * 60 * 60,
                pro_contact_limit: 50,
                trial_offer_escrow_stroops: 500_000,
                trial_offer_expiry_secs: 3_600,
            },
        ),
    );
    let client = ScoutAccessContractClient::new(&env, &id);

    let mut start = 1u32;
    while start <= TOTAL {
        let end = (start + 39).min(TOTAL);
        env.as_contract(&id, || {
            for idx in start..=end {
                env.storage()
                    .persistent()
                    .set(&DataKey::TrialOffer(PLAYER, idx), &true);
            }
        });
        start = end + 1;
    }
    env.as_contract(&id, || {
        let storage = env.storage().persistent();
        storage.set(&DataKey::TrialCounter(PLAYER), &TOTAL);
        storage.set(&DataKey::PlayerContacts(PLAYER), &true);
        storage.set(&DataKey::EvidenceAccessGrantCount(PLAYER), &true);
    });

    (env, client)
}

#[test]
fn purge_rejects_wrong_stale_and_out_of_range_cursors_and_completes_normally() {
    let (env, client) = setup();

    assert_eq!(
        client.try_purge_player_data(&PLAYER, &50),
        Err(Ok(ScoutAccessError::InvalidPurgeCursor))
    );
    assert_eq!(client.purge_player_data(&PLAYER, &0), (50, true));
    env.as_contract(&client.address, || {
        assert!(env
            .storage()
            .persistent()
            .has(&DataKey::TrialCounter(PLAYER)));
    });
    assert_eq!(
        client.try_purge_player_data(&PLAYER, &0),
        Err(Ok(ScoutAccessError::InvalidPurgeCursor))
    );
    assert_eq!(
        client.try_purge_player_data(&PLAYER, &121),
        Err(Ok(ScoutAccessError::InvalidPurgeCursor))
    );
    assert_eq!(client.purge_player_data(&PLAYER, &50), (100, true));
    assert_eq!(client.purge_player_data(&PLAYER, &100), (120, false));

    env.as_contract(&client.address, || {
        let storage = env.storage().persistent();
        assert!(!storage.has(&DataKey::TrialCounter(PLAYER)));
        assert!(!storage.has(&DataKey::PlayerContacts(PLAYER)));
        assert!(!storage.has(&DataKey::EvidenceAccessGrantCount(PLAYER)));
        assert!(!storage.has(&DataKey::PurgeCursor(PLAYER)));
        for idx in 1..=TOTAL {
            assert!(!storage.has(&DataKey::TrialOffer(PLAYER, idx)));
        }
    });
}
