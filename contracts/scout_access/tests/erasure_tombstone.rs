//! Tests for the erasure tombstone mechanism (issue #1622).
//!
//! Once a player is erased via `registration.deregister_player`, all player-scoped
//! writes must be permanently blocked with `PlayerErased` error.

use scoutchain_scout_access::{
    ScoutAccessContract, ScoutAccessContractClient, ScoutAccessError, SubscriptionTier,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn test_mark_player_erased_sets_tombstone() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire the registration contract
    client.set_registration_contract(&registration_addr);

    // Mark player as erased (authorized by registration contract)
    let player_id = 123u64;
    client.try_mark_player_erased(&player_id).unwrap();

    // Verify the tombstone is set
    let erased = env
        .storage()
        .persistent()
        .get::<scoutchain_scout_access::DataKey, bool>(&scoutchain_scout_access::DataKey::Erased(
            player_id,
        ))
        .unwrap();
    assert!(erased);
}

#[test]
fn test_mark_player_erased_requires_registration_auth() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let unauthorized = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire the registration contract
    client.set_registration_contract(&registration_addr);

    // Try to mark as erased with unauthorized caller
    let player_id = 123u64;
    env.mock_auths(&unauthorized, &[]);
    let result = client.try_mark_player_erased(&player_id);
    assert!(result.is_err());
}

#[test]
fn test_mark_player_erased_fails_when_registration_not_wired() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Try to mark as erased without wiring registration
    let player_id = 123u64;
    let result = client.try_mark_player_erased(&player_id);
    assert!(result.is_err());
}

#[test]
fn test_pay_to_contact_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let scout = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Set up XLM token and fee config
    let xlm_token = Address::generate(&env);
    client.initialize(&xlm_token, &admin);

    // Subscribe scout
    client.subscribe(&scout, &SubscriptionTier::Pro, &1000u64);

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to pay to contact - should fail with PlayerErased
    let result = client.try_pay_to_contact(&scout, &player_id);
    assert_eq!(result, Err(Ok(ScoutAccessError::PlayerErased)));
}

#[test]
fn test_batch_contact_players_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let scout = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Set up XLM token and fee config
    let xlm_token = Address::generate(&env);
    client.initialize(&xlm_token, &admin);

    // Subscribe scout
    client.subscribe(&scout, &SubscriptionTier::Pro, &1000u64);

    let player_id = 123u64;
    let mut player_ids = soroban_sdk::Vec::new(&env);
    player_ids.push_back(player_id);

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to batch contact - should fail with PlayerErased
    let result = client.try_batch_contact_players(&scout, &player_ids);
    assert_eq!(result, Err(Ok(ScoutAccessError::PlayerErased)));
}

#[test]
fn test_log_trial_offer_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let scout = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Set up XLM token and fee config
    let xlm_token = Address::generate(&env);
    client.initialize(&xlm_token, &admin);

    // Subscribe scout as Elite
    client.subscribe(&scout, &SubscriptionTier::Elite, &1000u64);

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to log trial offer - should fail with PlayerErased
    let result = client.try_log_trial_offer(
        &scout,
        &player_id,
        &soroban_sdk::String::from_str(&env, "bafytestcid"),
    );
    assert_eq!(result, Err(Ok(ScoutAccessError::PlayerErased)));
}

#[test]
fn test_admin_seed_contact_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let scout = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Open migration window
    client.open_migration_window();

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to seed contact - should fail with PlayerErased
    let contact = scoutchain_scout_access::types::ContactRecord {
        player_id,
        scout: scout.clone(),
        contacted_at: 100,
    };
    let result = client.try_admin_seed_contact(&contact);
    assert_eq!(result, Err(Ok(ScoutAccessError::PlayerErased)));
}

#[test]
fn test_admin_seed_trial_offer_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let scout = Address::generate(&env);

    let contract_id = env.register(ScoutAccessContract, (admin.clone(),));
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Open migration window
    client.open_migration_window();

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to seed trial offer - should fail with PlayerErased
    let offer = scoutchain_scout_access::types::TrialOffer {
        player_id,
        scout: scout.clone(),
        details_hash: soroban_sdk::String::from_str(&env, "bafytestcid"),
        logged_at: 100,
    };
    let result = client.try_admin_seed_trial_offer(&player_id, &1u32, &offer, &None);
    assert_eq!(result, Err(Ok(ScoutAccessError::PlayerErased)));
}
