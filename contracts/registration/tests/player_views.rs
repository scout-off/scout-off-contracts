//! Focused tests for the `is_player_deactivated` and `get_player_by_wallet`
//! public views on the registration contract.
//!
//! These views are consumed by other contracts and clients, so we exercise
//! them directly here rather than relying on indirect coverage.

use registration::{PlayerStatus, RegistrationContract, RegistrationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

/// Deploys a fresh registration contract and returns the env, client and
/// the contract address.
fn setup(env: &Env) -> (RegistrationContractClient<'_>, Address) {
    let contract_id = env.register_contract(None, RegistrationContract);
    let client = RegistrationContractClient::new(env, &contract_id);
    (client, contract_id)
}

/// Registers a player and returns their id.
fn register_player(env: &Env, client: &RegistrationContractClient<'_>) -> (u64, Address) {
    let wallet = Address::generate(env);
    let id = client.register_player(&wallet);
    (id, wallet)
}

// ---------------------------------------------------------------------------
// is_player_deactivated
// ---------------------------------------------------------------------------

#[test]
fn is_player_deactivated_returns_false_for_active_player() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let (id, _wallet) = register_player(&env, &client);

    assert!(!client.is_player_deactivated(&id));
}

#[test]
fn is_player_deactivated_returns_true_after_deactivation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let (id, _wallet) = register_player(&env, &client);
    client.deactivate_player(&id);

    assert!(client.is_player_deactivated(&id));
}

#[test]
fn is_player_deactivated_returns_false_after_reactivation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let (id, _wallet) = register_player(&env, &client);
    client.deactivate_player(&id);
    assert!(client.is_player_deactivated(&id));

    client.reactivate_player(&id);
    assert!(!client.is_player_deactivated(&id));
}

#[test]
fn is_player_deactivated_unknown_id_returns_false() {
    // Documented behaviour: an unknown player id is treated as not
    // deactivated (the view returns `false` rather than erroring).
    // NOTE: if the contract is changed to return `PlayerNotFound` for
    // unknown ids, update this test and the doc comment on the view.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let unknown_id = 9999u64;

    assert!(!client.is_player_deactivated(&unknown_id));
}

// ---------------------------------------------------------------------------
// get_player_by_wallet
// ---------------------------------------------------------------------------

#[test]
fn get_player_by_wallet_returns_profile_matching_get_player() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let (id, wallet) = register_player(&env, &client);

    let by_wallet = client.get_player_by_wallet(&wallet);
    let by_id = client.get_player(&id);

    assert_eq!(by_wallet, by_id);
    assert_eq!(by_wallet.id, id);
    assert_eq!(by_wallet.wallet, wallet);
    assert_eq!(by_wallet.status, PlayerStatus::Active);
}

#[test]
fn get_player_by_wallet_unknown_wallet_returns_player_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let unknown_wallet = Address::generate(&env);

    let result = client.try_get_player_by_wallet(&unknown_wallet);
    assert_eq!(result, Err(Ok(registration::Error::PlayerNotFound)));
}

#[test]
fn get_player_by_wallet_returns_player_not_found_after_deregistration() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    let (id, wallet) = register_player(&env, &client);

    // Sanity check: the wallet resolves before deregistration.
    assert_eq!(client.get_player_by_wallet(&wallet).id, id);

    client.deregister_player(&id);

    let result = client.try_get_player_by_wallet(&wallet);
    assert_eq!(result, Err(Ok(registration::Error::PlayerNotFound)));
}
