//! Tests for the erasure tombstone mechanism (issue #1622).
//!
//! Once a player is erased via `registration.deregister_player`, all player-scoped
//! writes must be permanently blocked with `PlayerErased` error.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressError, ProgressEntry};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as_, Address, Env, BytesN};

#[test]
fn test_mark_player_erased_sets_tombstone() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &contract_id);

    // Wire the registration contract
    client.set_registration_contract(&registration_addr);

    // Mark player as erased (authorized by registration contract)
    let player_id = 123u64;
    client
        .try_mark_player_erased(&player_id)
        .unwrap();

    // Verify the tombstone is set
    let erased = env
        .storage()
        .persistent()
        .get::<scoutchain_progress::DataKey, bool>(&scoutchain_progress::DataKey::Erased(player_id))
        .unwrap();
    assert!(erased);
}

#[test]
fn test_mark_player_erased_requires_registration_auth() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let unauthorized = Address::generate(&env);

    let contract_id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &contract_id);

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

    let contract_id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &contract_id);

    // Try to mark as erased without wiring registration
    let player_id = 123u64;
    let result = client.try_mark_player_erased(&player_id);
    assert!(result.is_err());
}

#[test]
fn test_reset_player_level_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to reset level - should fail with PlayerErased
    let result = client.try_reset_player_level(&player_id, &ProgressLevel::Unverified);
    assert_eq!(result, Err(Ok(ProgressError::PlayerErased)));
}

#[test]
fn test_admin_seed_history_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Open migration window
    client.open_migration_window();

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to seed history - should fail with PlayerErased
    let entry = ProgressEntry {
        player_id,
        old_level: ProgressLevel::Unverified,
        new_level: ProgressLevel::VerifiedIdentity,
        updated_by: admin.clone(),
        updated_at: 100,
        milestone_ref: 1,
        ledger_sequence: 1,
    };
    let result = client.try_admin_seed_history(&player_id, &1u32, &entry, &None::<BytesN<32>>);
    assert_eq!(result, Err(Ok(ProgressError::PlayerErased)));
}

#[test]
fn test_restore_player_level_record_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(ProgressContract, (admin.clone(),));
    let client = ProgressContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to restore record - should fail with PlayerErased
    let result = client.try_restore_player_level_record(&player_id);
    assert_eq!(result, Err(Ok(ProgressError::PlayerErased)));
}
