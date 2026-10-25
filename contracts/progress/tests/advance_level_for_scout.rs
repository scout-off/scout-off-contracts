//! `advance_level_for_scout` authorisation (issue #1640).
//!
//! The scout address is attribution only; auth must come from the
//! whitelisted ScoutAccessContract, so a scout calling directly fails.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressError};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn fails_closed_without_scout_access_wired() {
    let env = Env::default();
    env.mock_all_auths();
    let client = ProgressContractClient::new(&env, &env.register(ProgressContract, ()));
    client.initialize(&Address::generate(&env));
    assert_eq!(
        client.try_advance_level_for_scout(&Address::generate(&env), &1u64, &1u32),
        Err(Ok(ProgressError::NotInitialized))
    );
}

#[test]
fn direct_call_by_scout_without_contract_auth_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = ProgressContractClient::new(&env, &env.register(ProgressContract, ()));
    client.initialize(&Address::generate(&env));
    let scout_access = Address::generate(&env);
    client.set_scout_access_contract(&scout_access);

    // Only the scout signs — the scout_access contract's auth is missing.
    env.set_auths(&[]);
    let scout = Address::generate(&env);
    assert!(client
        .try_advance_level_for_scout(&scout, &1u64, &1u32)
        .is_err());
}
