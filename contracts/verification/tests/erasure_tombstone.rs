//! Tests for the erasure tombstone mechanism (issue #1622).
//!
//! Once a player is erased via `registration.deregister_player`, all player-scoped
//! writes must be permanently blocked with `PlayerErased` error.

use scoutchain_shared_types::ValidatorStatus;
use scoutchain_verification::{
    VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{testutils::Address as_, Address, Env, String};

#[test]
fn test_mark_player_erased_sets_tombstone() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

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
        .get::<scoutchain_verification::DataKey, bool>(&scoutchain_verification::DataKey::Erased(player_id))
        .unwrap();
    assert!(erased);
}

#[test]
fn test_mark_player_erased_requires_registration_auth() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let unauthorized = Address::generate(&env);

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

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

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

    // Try to mark as erased without wiring registration
    let player_id = 123u64;
    let result = client.try_mark_player_erased(&player_id);
    assert!(result.is_err());
}

#[test]
fn test_approve_milestone_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let validator = Address::generate(&env);

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Register a validator
    client.register_validator(
        &validator,
        &String::from_str(&env, "UEFA B License"),
        &String::from_str(&env, "FC Example Academy"),
    );

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to approve milestone - should fail with PlayerErased
    let result = client.try_approve_milestone(
        &validator,
        &player_id,
        &String::from_str(&env, "Good performance"),
        &String::from_str(&env, "bafytestcid"),
        &None::<String>,
    );
    assert_eq!(result, Err(Ok(VerificationError::PlayerErased)));
}

#[test]
fn test_dispute_milestone_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);
    let validator = Address::generate(&env);

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to dispute milestone - should fail with PlayerErased
    let result = client.try_dispute_milestone(
        &validator,
        &player_id,
        &1u32,
        &String::from_str(&env, "Invalid evidence"),
        &50u32,
    );
    assert_eq!(result, Err(Ok(VerificationError::PlayerErased)));
}

#[test]
fn test_admin_seed_milestone_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    // Open migration window
    client.open_migration_window();

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to seed milestone - should fail with PlayerErased
    let milestone = scoutchain_verification::types::Milestone {
        player_id,
        validator: admin.clone(),
        description: String::from_str(&env, "Test milestone"),
        evidence_hash: String::from_str(&env, "bafytestcid"),
        approved_at: 100,
        ledger_sequence: 1,
    };
    let result = client.try_admin_seed_milestone(&player_id, &1u32, &milestone, &admin);
    assert_eq!(result, Err(Ok(VerificationError::PlayerErased)));
}

#[test]
fn test_restore_milestone_record_rejects_erased_player() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registration_addr = Address::generate(&env);

    let contract_id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &contract_id);

    // Wire registration
    client.set_registration_contract(&registration_addr);

    let player_id = 123u64;

    // Mark player as erased
    client.mark_player_erased(&player_id);

    // Try to restore record - should fail with PlayerErased
    let result = client.try_restore_milestone_record(&player_id, &1u32);
    assert_eq!(result, Err(Ok(VerificationError::PlayerErased)));
}
