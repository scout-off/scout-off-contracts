use scoutchain_verification::{
    DataKey, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

const PLAYER: u64 = 7;
const TOTAL: u32 = 120;

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(VerificationContract, (admin,));
    let client = VerificationContractClient::new(&env, &id);

    let mut start = 0u32;
    while start < TOTAL {
        let end = (start + 39).min(TOTAL - 1);
        env.as_contract(&id, || {
            for idx in start..=end {
                env.storage()
                    .persistent()
                    .set(&DataKey::Milestone(PLAYER, idx), &true);
            }
        });
        start = end + 1;
    }
    env.as_contract(&id, || {
        let storage = env.storage().persistent();
        storage.set(&DataKey::MilestoneCounter(PLAYER), &TOTAL);
        storage.set(&DataKey::PlayerAffiliations(PLAYER), &true);
        storage.set(&DataKey::PlayerDisputes(PLAYER), &true);
    });

    (env, client)
}

#[test]
fn purge_rejects_wrong_stale_and_out_of_range_cursors_and_completes_normally() {
    let (env, client) = setup();

    assert_eq!(
        client.try_purge_player_data(&PLAYER, &50),
        Err(Ok(VerificationError::InvalidPurgeCursor))
    );
    assert_eq!(client.purge_player_data(&PLAYER, &0), (50, true));
    env.as_contract(&client.address, || {
        let storage = env.storage().persistent();
        assert!(storage.has(&DataKey::PlayerAffiliations(PLAYER)));
        assert!(storage.has(&DataKey::PlayerDisputes(PLAYER)));
    });
    assert_eq!(
        client.try_purge_player_data(&PLAYER, &0),
        Err(Ok(VerificationError::InvalidPurgeCursor))
    );
    assert_eq!(
        client.try_purge_player_data(&PLAYER, &121),
        Err(Ok(VerificationError::InvalidPurgeCursor))
    );
    assert_eq!(client.purge_player_data(&PLAYER, &50), (100, true));
    assert_eq!(client.purge_player_data(&PLAYER, &100), (120, false));

    env.as_contract(&client.address, || {
        let storage = env.storage().persistent();
        assert!(!storage.has(&DataKey::MilestoneCounter(PLAYER)));
        assert!(!storage.has(&DataKey::PlayerAffiliations(PLAYER)));
        assert!(!storage.has(&DataKey::PlayerDisputes(PLAYER)));
        assert!(!storage.has(&DataKey::PurgeCursor(PLAYER)));
        for idx in 0..TOTAL {
            assert!(!storage.has(&DataKey::Milestone(PLAYER, idx)));
        }
    });
}
