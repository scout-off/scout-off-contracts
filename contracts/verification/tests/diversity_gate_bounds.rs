//! Bounds and TTL coverage for the diversity / region gates (issue #1632).

use scoutchain_shared_types::testutils::has_event;
use scoutchain_verification::{
    DataKey, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    testutils::{storage::Persistent as _, Address as _, Ledger as _},
    Address, Env, String, Vec,
};

const MAX_VALIDATORS: u32 = 100;

fn setup() -> (Env, Address, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    client.initialize(&Address::generate(&env));
    (env, contract_id, client)
}

fn register(env: &Env, client: &VerificationContractClient) {
    client.register_validator(
        &Address::generate(env),
        &String::from_str(env, "UEFA-B-License-2026"),
        &String::from_str(env, "Academy"),
        &Vec::new(env),
    );
}

fn unreachable_emitted(env: &Env) -> bool {
    has_event(env, "diversity_gate_unreachable")
}

#[test]
fn region_quorum_above_max_is_rejected() {
    let (_env, _, client) = setup();
    assert_eq!(
        client.try_set_min_region_quorum(&(MAX_VALIDATORS + 1)),
        Err(Ok(VerificationError::InvalidRegionQuorum))
    );
}

#[test]
fn region_quorum_zero_disables_without_warning() {
    let (env, _, client) = setup();
    client.set_min_region_quorum(&0u32);
    assert!(!unreachable_emitted(&env));
    assert_eq!(client.get_min_region_quorum(), 0);
}

#[test]
fn region_quorum_unreachable_emits_warning() {
    let (env, _, client) = setup();
    register(&env, &client);
    client.set_min_region_quorum(&MAX_VALIDATORS);
    assert!(unreachable_emitted(&env));
    assert_eq!(client.get_min_region_quorum(), MAX_VALIDATORS);
}

#[test]
fn region_quorum_reachable_does_not_warn() {
    let (env, _, client) = setup();
    register(&env, &client);
    register(&env, &client);
    client.set_min_region_quorum(&2u32);
    assert!(!unreachable_emitted(&env));
}

#[test]
fn diversity_config_rejects_zero_affiliations() {
    let (_env, _, client) = setup();
    assert_eq!(
        client.try_set_diversity_config(&0u32, &1u32),
        Err(Ok(VerificationError::InvalidDiversityConfig))
    );
}

#[test]
fn diversity_config_rejects_above_max() {
    let (_env, _, client) = setup();
    assert_eq!(
        client.try_set_diversity_config(&(MAX_VALIDATORS + 1), &1u32),
        Err(Ok(VerificationError::InvalidDiversityConfig))
    );
}

#[test]
fn diversity_config_rejects_zero_starting_index() {
    let (_env, _, client) = setup();
    assert_eq!(
        client.try_set_diversity_config(&1u32, &0u32),
        Err(Ok(VerificationError::InvalidDiversityConfig))
    );
}

#[test]
fn diversity_config_unreachable_emits_warning() {
    let (env, _, client) = setup();
    register(&env, &client);
    client.set_diversity_config(&2u32, &1u32);
    assert!(unreachable_emitted(&env));
}

#[test]
fn diversity_config_reachable_does_not_warn() {
    let (env, _, client) = setup();
    register(&env, &client);
    register(&env, &client);
    client.set_diversity_config(&2u32, &1u32);
    assert!(!unreachable_emitted(&env));
}

#[test]
fn diversity_config_survives_long_idle_periods() {
    let (env, contract_id, client) = setup();
    client.set_diversity_config(&1u32, &1u32);
    let ttl = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get_ttl(&DataKey::DiversityConfig)
    });
    assert!(ttl >= 500, "write must extend TTL, got {ttl}");

    // Advance well past the default minimum persistent TTL.
    env.ledger().with_mut(|l| l.sequence_number += 100_000);
    assert!(client.get_diversity_config().is_some());

    // Read extends again; the entry stays alive after another long gap.
    env.ledger().with_mut(|l| l.sequence_number += 100_000);
    assert!(client.get_diversity_config().is_some());
}
