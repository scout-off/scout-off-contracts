//! Admin registration-cooldown setter/getter tests for the registration
//! contract (#1539).
//!
//! `register_player` and `register_scout` reject a re-registration of the same
//! wallet while its cooldown window has not elapsed, returning
//! `RegistrationCooldown` (code 16). The window is admin-configurable within
//! `0..=604_800` seconds; out-of-range values return `InvalidCooldown`.

use scoutchain_registration::{RegistrationContract, RegistrationContractClient, ScoutChainError};
use scoutchain_shared_types::testutils::last_event_data;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
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
