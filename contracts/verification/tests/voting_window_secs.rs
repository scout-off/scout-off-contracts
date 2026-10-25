//! Tests for `set_voting_window_secs` / `get_voting_window_secs` — issue #1699.
//!
//! Accepted range is `MIN_VOTING_WINDOW_SECS..=MAX_VOTING_WINDOW_SECS`
//! (1 hour to 90 days); default is 14 days.

use scoutchain_verification::{
    VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env, IntoVal, TryFromVal,
};

const MIN_WINDOW: u64 = 3_600; // MIN_VOTING_WINDOW_SECS (1 hour)
const MAX_WINDOW: u64 = 7_776_000; // MAX_VOTING_WINDOW_SECS (90 days)
const DEFAULT_WINDOW: u64 = 1_209_600; // DEFAULT_VOTING_WINDOW_SECS (14 days)

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    client.initialize(&Address::generate(&env));
    (env, client)
}

#[test]
fn voting_window_default_before_any_set() {
    let (_env, client) = setup();
    assert_eq!(client.get_voting_window_secs(), DEFAULT_WINDOW);
}

#[test]
fn voting_window_below_min_rejected() {
    let (_env, client) = setup();
    let result = client.try_set_voting_window_secs(&(MIN_WINDOW - 1));
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
    assert_eq!(client.get_voting_window_secs(), DEFAULT_WINDOW);
}

#[test]
fn voting_window_above_max_rejected() {
    let (_env, client) = setup();
    let result = client.try_set_voting_window_secs(&(MAX_WINDOW + 1));
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
    assert_eq!(client.get_voting_window_secs(), DEFAULT_WINDOW);
}

#[test]
fn voting_window_min_accepted() {
    let (_env, client) = setup();
    client.set_voting_window_secs(&MIN_WINDOW);
    assert_eq!(client.get_voting_window_secs(), MIN_WINDOW);
}

#[test]
fn voting_window_max_accepted() {
    let (_env, client) = setup();
    client.set_voting_window_secs(&MAX_WINDOW);
    assert_eq!(client.get_voting_window_secs(), MAX_WINDOW);
}

#[test]
fn voting_window_update_emits_old_and_new_values() {
    let (env, client) = setup();
    client.set_voting_window_secs(&MIN_WINDOW);

    let all = env.events().all();
    let event = all
        .events()
        .iter()
        .find_map(|e| match &e.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => match v0.topics.first() {
                Some(soroban_sdk::xdr::ScVal::Symbol(sym))
                    if AsRef::<[u8]>::as_ref(&sym.0) == b"voting_window_secs_updated" =>
                {
                    Some(v0.clone())
                }
                _ => None,
            },
        })
        .expect("voting_window_secs_updated event must be emitted");

    let expected: soroban_sdk::Val = (DEFAULT_WINDOW, MIN_WINDOW).into_val(&env);
    let expected_xdr = soroban_sdk::xdr::ScVal::try_from_val(&env, &expected).unwrap();
    assert_eq!(event.data, expected_xdr);
}

#[test]
fn voting_window_non_admin_rejected() {
    let (env, client) = setup();
    // Drop the blanket auth mock: the admin has not signed this call.
    env.set_auths(&[]);
    assert!(client.try_set_voting_window_secs(&MIN_WINDOW).is_err());
    assert_eq!(client.get_voting_window_secs(), DEFAULT_WINDOW);
}
