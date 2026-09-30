//! TTL coverage test for issue #1401.
//!
//! Verifies that `commit_approved_milestone` extends the four persistent keys
//! (`ValidatorPlayerMilestoneCount`, `ValidatorPlayers`, `ValidatorMilestones`,
//! `PlayerAffiliations`) with the full `PERSISTENT_TTL_MAX` (518,400 ledgers).
//!
//! Without these extensions the keys receive the network's default minimal TTL
//! and risk archival while the milestones they index are still active.

use scoutchain_verification::{
    DataKey, VerificationContract, VerificationContractClient,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

const CREDENTIALS: &str = "UEFA-B-License-2026";
const VALID_CID: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";

/// Returns the measured TTL for a key, or 0 if the key does not exist.
fn get_ttl(env: &Env, contract_id: &Address, key: &DataKey) -> u32 {
    env.as_contract(contract_id, || env.storage().persistent().get_ttl(key))
}

#[test]
fn test_commit_approved_milestone_extends_all_related_keys() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Register two validators with different affiliations so PlayerAffiliations
    // accumulates more than one entry.
    let validator1 = Address::generate(&env);
    client.register_validator(
        &validator1,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, "Academy A"),
        &Vec::new(&env),
    );

    let validator2 = Address::generate(&env);
    client.register_validator(
        &validator2,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, "Academy B"),
        &Vec::new(&env),
    );

    let player_id: u64 = 1;

    // ── Approve first milestone from validator1 ─────────────────────────────
    client.approve_milestone(
        &validator1,
        &player_id,
        &String::from_str(&env, "first milestone"),
        &String::from_str(&env, VALID_CID),
        &None,
    );

    // All four keys should now exist with TTL close to PERSISTENT_TTL_MAX.
    const PERSISTENT_TTL_MAX: u32 = 518_400;
    const EPSILON: u32 = 100; // Allow small ledger-count slippage

    let vpmc = get_ttl(
        &env,
        &contract_id,
        &DataKey::ValidatorPlayerMilestoneCount(validator1.clone(), player_id),
    );
    assert!(
        vpmc >= PERSISTENT_TTL_MAX - EPSILON,
        "ValidatorPlayerMilestoneCount TTL {} is too low (expected >= {})",
        vpmc,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    let vp = get_ttl(
        &env,
        &contract_id,
        &DataKey::ValidatorPlayers(validator1.clone()),
    );
    assert!(
        vp >= PERSISTENT_TTL_MAX - EPSILON,
        "ValidatorPlayers TTL {} is too low (expected >= {})",
        vp,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    let vm = get_ttl(
        &env,
        &contract_id,
        &DataKey::ValidatorMilestones(validator1.clone()),
    );
    assert!(
        vm >= PERSISTENT_TTL_MAX - EPSILON,
        "ValidatorMilestones TTL {} is too low (expected >= {})",
        vm,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    let pa = get_ttl(
        &env,
        &contract_id,
        &DataKey::PlayerAffiliations(player_id),
    );
    assert!(
        pa >= PERSISTENT_TTL_MAX - EPSILON,
        "PlayerAffiliations TTL {} is too low (expected >= {})",
        pa,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    // ── Approve a second milestone from validator2 for the same player ──────
    let cid2 = "QmvwxyzABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdef";
    client.approve_milestone(
        &validator2,
        &player_id,
        &String::from_str(&env, "second milestone"),
        &String::from_str(&env, cid2),
        &None,
    );

    // PlayerAffiliations should now have both affiliations.
    let pa_after = get_ttl(
        &env,
        &contract_id,
        &DataKey::PlayerAffiliations(player_id),
    );
    assert!(
        pa_after >= PERSISTENT_TTL_MAX - EPSILON,
        "PlayerAffiliations TTL {} is too low after second approval (expected >= {})",
        pa_after,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    // Validator2's keys should also be extended.
    let vpmc2 = get_ttl(
        &env,
        &contract_id,
        &DataKey::ValidatorPlayerMilestoneCount(validator2.clone(), player_id),
    );
    assert!(
        vpmc2 >= PERSISTENT_TTL_MAX - EPSILON,
        "ValidatorPlayerMilestoneCount (v2) TTL {} is too low (expected >= {})",
        vpmc2,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    let vp2 = get_ttl(
        &env,
        &contract_id,
        &DataKey::ValidatorPlayers(validator2.clone()),
    );
    assert!(
        vp2 >= PERSISTENT_TTL_MAX - EPSILON,
        "ValidatorPlayers (v2) TTL {} is too low (expected >= {})",
        vp2,
        PERSISTENT_TTL_MAX - EPSILON,
    );

    let vm2 = get_ttl(
        &env,
        &contract_id,
        &DataKey::ValidatorMilestones(validator2.clone()),
    );
    assert!(
        vm2 >= PERSISTENT_TTL_MAX - EPSILON,
        "ValidatorMilestones (v2) TTL {} is too low (expected >= {})",
        vm2,
        PERSISTENT_TTL_MAX - EPSILON,
    );
}