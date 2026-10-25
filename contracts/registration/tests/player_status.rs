//! Tests for `get_player_status` — issue #1697.
//!
//! Returns `Active` / `Deactivated` for an existing player and
//! `PlayerNotFound` otherwise.

use scoutchain_registration::{
    PlayerStatus, PlayerVitals, RegistrationContract, RegistrationContractClient, ScoutChainError,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

struct Harness {
    env: Env,
    client: RegistrationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &id);
    client.initialize(&admin);
    Harness { env, client }
}

fn register(h: &Harness) -> u64 {
    let vitals = PlayerVitals {
        age: 20,
        position: String::from_str(&h.env, "ST"),
        region: String::from_str(&h.env, "EU"),
        nationality: String::from_str(&h.env, "FR"),
    };
    let mut hashes = Vec::new(&h.env);
    hashes.push_back(String::from_str(&h.env, "bafytestcid"));
    h.client
        .register_player(&Address::generate(&h.env), &vitals, &hashes)
}

#[test]
fn test_player_status_fresh_player_is_active() {
    let h = setup();
    let id = register(&h);
    assert_eq!(h.client.get_player_status(&id), PlayerStatus::Active);
}

#[test]
fn test_player_status_deactivate_then_reactivate() {
    let h = setup();
    let id = register(&h);

    h.client.deactivate_player(&id);
    assert_eq!(h.client.get_player_status(&id), PlayerStatus::Deactivated);

    h.client.reactivate_player(&id);
    assert_eq!(h.client.get_player_status(&id), PlayerStatus::Active);
}

#[test]
fn test_player_status_unknown_id_not_found() {
    let h = setup();
    assert_eq!(
        h.client.try_get_player_status(&999),
        Err(Ok(ScoutChainError::PlayerNotFound))
    );
}

#[test]
fn test_player_status_after_deregister_not_found() {
    let h = setup();
    let id = register(&h);
    h.client.deregister_player(&id);
    assert_eq!(
        h.client.try_get_player_status(&id),
        Err(Ok(ScoutChainError::PlayerNotFound))
    );
}
