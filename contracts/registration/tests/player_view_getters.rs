//! Focused tests for the public views `is_player_deactivated` and
//! `get_player_by_wallet` — issue #1880.

use scoutchain_registration::{
    PlayerVitals, RegistrationContract, RegistrationContractClient, ScoutChainError,
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
    let id = env.register(RegistrationContract, (admin,));
    let client = RegistrationContractClient::new(&env, &id);
    Harness { env, client }
}

fn register(h: &Harness) -> (u64, Address) {
    let wallet = Address::generate(&h.env);
    let vitals = PlayerVitals {
        age: 20,
        position: String::from_str(&h.env, "ST"),
        region: String::from_str(&h.env, "EU"),
        nationality: String::from_str(&h.env, "FR"),
    };
    let mut hashes = Vec::new(&h.env);
    hashes.push_back(String::from_str(&h.env, "bafytestcid"));
    let id = h.client.register_player(&wallet, &vitals, &hashes);
    (id, wallet)
}

// ---------------------------------------------------------------------------
// is_player_deactivated
// ---------------------------------------------------------------------------

#[test]
fn is_player_deactivated_false_for_active_player() {
    let h = setup();
    let (id, _) = register(&h);
    assert!(!h.client.is_player_deactivated(&id));
}

#[test]
fn is_player_deactivated_follows_deactivate_and_reactivate() {
    let h = setup();
    let (id, _) = register(&h);

    h.client.deactivate_player(&id);
    assert!(h.client.is_player_deactivated(&id));

    h.client.reactivate_player(&id);
    assert!(!h.client.is_player_deactivated(&id));
}

/// Documented behaviour: the view never errors; an id that was never
/// registered is simply reported as not deactivated (`false`).
#[test]
fn is_player_deactivated_false_for_unknown_id() {
    let h = setup();
    assert!(!h.client.is_player_deactivated(&9_999));
}

#[test]
fn is_player_deactivated_is_per_player() {
    let h = setup();
    let (a, _) = register(&h);
    let (b, _) = register(&h);

    h.client.deactivate_player(&a);
    assert!(h.client.is_player_deactivated(&a));
    assert!(!h.client.is_player_deactivated(&b));
}

#[test]
fn is_player_deactivated_cleared_after_deregister() {
    let h = setup();
    let (id, _) = register(&h);
    h.client.deactivate_player(&id);
    h.client.deregister_player(&id);
    assert!(!h.client.is_player_deactivated(&id));
}

// ---------------------------------------------------------------------------
// get_player_by_wallet
// ---------------------------------------------------------------------------

#[test]
fn get_player_by_wallet_matches_get_player() {
    let h = setup();
    let (id, wallet) = register(&h);

    let by_wallet = h.client.get_player_by_wallet(&wallet);
    let direct = h.client.get_player(&id);
    assert_eq!(by_wallet.player_id, direct.player_id);
    assert_eq!(by_wallet.wallet, direct.wallet);
    assert_eq!(by_wallet.vitals, direct.vitals);
    assert_eq!(by_wallet.ipfs_hashes, direct.ipfs_hashes);
    assert_eq!(by_wallet.level, direct.level);
    assert_eq!(by_wallet.registered_at, direct.registered_at);
    assert_eq!(by_wallet.updated_at, direct.updated_at);
    assert_eq!(by_wallet.player_id, id);
    assert_eq!(by_wallet.wallet, wallet);
}

#[test]
fn get_player_by_wallet_unknown_wallet_is_player_not_found() {
    let h = setup();
    let stranger = Address::generate(&h.env);
    let res = h.client.try_get_player_by_wallet(&stranger);
    assert!(matches!(res, Err(Ok(ScoutChainError::PlayerNotFound))));
}

#[test]
fn get_player_by_wallet_after_deregister_is_player_not_found() {
    let h = setup();
    let (id, wallet) = register(&h);
    h.client.deregister_player(&id);
    let res = h.client.try_get_player_by_wallet(&wallet);
    assert!(matches!(res, Err(Ok(ScoutChainError::PlayerNotFound))));
}
