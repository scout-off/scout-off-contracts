//! Admin registration-cooldown setter/getter tests for the registration
//! contract (#1539).
//!
//! `register_player` and `register_scout` reject a re-registration of the same
//! wallet while its cooldown window has not elapsed, returning
//! `RegistrationCooldown` (code 16). The window is admin-configurable within
//! `0..=604_800` seconds; out-of-range values return `InvalidCooldown`.
//!
//! Guard-order tests (#1660) pin that both entrypoints share the same order:
//! pause -> init -> auth -> cooldown -> duplicate -> input validation. A wallet
//! in cooldown that also submits an invalid region therefore gets
//! `RegistrationCooldown` from both `register_player` and `register_scout`.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient, ScoutChainError};
use scoutchain_shared_types::testutils::last_event_data;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, String,
};

const START: u64 = 1_000_000;
const COOLDOWN_SECS: u64 = 3_600; // 1 hour

fn setup() -> (Env, RegistrationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let contract_id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

#[test]
fn default_cooldown_is_24h() {
    let (_env, client) = setup();
    assert_eq!(client.get_reg_cooldown(), 86_400);
}

#[test]
fn set_cooldown_to_zero_disables() {
    let (_env, client) = setup();
    client.set_reg_cooldown(&0);
    assert_eq!(client.get_reg_cooldown(), 0);
}

#[test]
fn set_cooldown_to_n_enforces_n() {
    let (_env, client) = setup();
    client.set_reg_cooldown(&COOLDOWN_SECS);
    assert_eq!(client.get_reg_cooldown(), COOLDOWN_SECS);
}

#[test]
fn set_cooldown_exceeds_max_rejected() {
    let (_env, client) = setup();
    assert_eq!(
        client.try_set_reg_cooldown(&604_801),
        Err(Ok(ScoutChainError::InvalidCooldown))
    );
    assert_eq!(client.get_reg_cooldown(), 86_400);
}

#[test]
fn set_cooldown_max_accepted() {
    let (_env, client) = setup();
    client.set_reg_cooldown(&604_800);
    assert_eq!(client.get_reg_cooldown(), 604_800);
}

#[test]
fn non_admin_cannot_set_cooldown() {
    let (env, client) = setup();
    let non_admin = Address::generate(&env);
    // Only `non_admin` signs; the stored admin's auth is not satisfied.
    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_reg_cooldown",
            args: (3_600u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_set_reg_cooldown(&3_600).is_err());
}

#[test]
fn reg_cooldown_event_emitted() {
    let (env, client) = setup();
    client.set_reg_cooldown(&COOLDOWN_SECS);
    let data = last_event_data(&env, "reg_cooldown_updated").expect("event emitted");
    let (old, new): (u64, u64) = data.into_val(&env);
    assert_eq!((old, new), (86_400, COOLDOWN_SECS));
}

#[test]
fn get_scout_verification_extends_scout_ttl() {
    let (env, client) = setup();
    let scout = Address::generate(&env);
    client.register_scout(&scout);

    // Read only via `get_scout_verification` to exercise its TTL extension.
    let _ = client.get_scout_verification(&scout);

    // Advance the ledger well past the default entry TTL.
    env.ledger().with_mut(|l| l.sequence_number += 100_000);

    // The scout record must still be readable thanks to the extension.
    let profile = client.get_scout(&scout);
    assert_eq!(profile.address, scout);
}

/// A wallet in cooldown that also submits an invalid region must receive the
/// same error (`RegistrationCooldown`) from both entrypoints, because cooldown
/// is checked before input validation in both.
#[test]
fn cooldown_wins_over_invalid_region_for_both_entrypoints() {
    let (env, client) = setup();
    client.set_reg_cooldown(&COOLDOWN_SECS);

    let wallet = Address::generate(&env);
    let invalid_region = String::from_str(&env, "not-a-valid-region");

    // First registration succeeds and starts the cooldown window.
    client.register_player(&wallet, &invalid_region);

    // Still inside the cooldown window: cooldown must win over the invalid
    // region for both entrypoints.
    assert_eq!(
        client.try_register_player(&wallet, &invalid_region),
        Err(Ok(ScoutChainError::RegistrationCooldown))
    );
    assert_eq!(
        client.try_register_scout(&wallet, &invalid_region),
        Err(Ok(ScoutChainError::RegistrationCooldown))
    );
}

/// Pins the storage key of the cooldown (#1725). `DataKey::RegCooldownSecs`
/// serializes as `[Symbol("RegCooldownSecs"), U64(0)]`; a refactor that drops
/// or changes the dummy `0` discriminator would orphan the value stored on
/// deployed contracts, so this test must fail loudly if the key moves.
#[test]
fn reg_cooldown_storage_key_is_pinned() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &contract_id);
    client.initialize(&Address::generate(&env));
    client.set_reg_cooldown(&1_234);
    let key = (soroban_sdk::Symbol::new(&env, "RegCooldownSecs"), 0u64);
    let stored: Option<u64> =
        env.as_contract(&contract_id, || env.storage().instance().get(&key));
    assert_eq!(stored, Some(1_234));
}
