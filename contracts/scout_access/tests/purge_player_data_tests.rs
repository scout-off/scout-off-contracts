//! Unit tests for `scout_access::purge_player_data` basic behaviour (#1891).
//!
//! These pin the *current* behaviour for the small (< 50 trial offers) case.
//! The >50-offers bug and the escrow-stranding bug have their own issues and
//! regression tests; do not extend these tests to cover them.

mod common;

use common::{wire_registration, RegistrationStub};
use scout_access::{
    ContractError, DataKey, PlayerContact, ScoutAccessContract, ScoutAccessContractClient,
};
use soroban_sdk::{
    testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke},
    token, Address, Env, IntoVal, Symbol, TryFromVal,
};

/// Standard test setup mirroring `admin_transfer_properties.rs`:
/// register the contract, wire a registration stub, configure fees and
/// register a Stellar asset contract for the escrow token.
fn setup<'a>() -> (Env, ScoutAccessContractClient<'a>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, ScoutAccessContract);
    let client = ScoutAccessContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let player = Address::generate(&env);

    // Registration stub used by the contract to resolve player -> wallet.
    let reg_id = env.register_stellar_asset_contract_v2(admin.clone());
    let _ = reg_id;

    let stub = RegistrationStub::new(&env);
    stub.map_player(&player, &player);
    wire_registration(&env, &contract_id, &stub);

    // Fee configuration (same shape as admin_transfer_properties.rs).
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_address = token_id.address();
    let token_admin_client = token::StellarAssetClient::new(&env, &token_address);
    token_admin_client.mint(&admin, &1_000_000);

    client.initialize(&admin, &token_address);

    (env, client, admin, player, token_address)
}

/// Create `count` trial offers for `player`, each with a small escrow.
fn create_trial_offers(
    env: &Env,
    client: &ScoutAccessContractClient,
    admin: &Address,
    player: &Address,
    count: u32,
) {
    for i in 0..count {
        let amount: i128 = 100 + i as i128;
        client.create_trial_offer(admin, player, &amount);
    }
    let _ = env;
}

#[test]
fn purge_refused_while_unexpired_escrow_exists() {
    let (env, client, admin, player, _token) = setup();
    create_trial_offers(&env, &client, &admin, &player, 3);

    // Current behaviour: an unexpired escrow causes the purge to be refused
    // with `TrialOfferNotFound`. Update this assertion when the dedicated
    // error lands.
    let result = client.try_purge_player_data(&admin, &player);
    assert_eq!(result, Err(Ok(ContractError::TrialOfferNotFound)));

    // Nothing was removed.
    assert_eq!(client.get_trial_count(&player), 3);
}

#[test]
fn purge_succeeds_after_expiry_refunds() {
    let (env, client, admin, player, _token) = setup();
    create_trial_offers(&env, &client, &admin, &player, 3);

    // Advance time so the offers expire, then refund them.
    env.ledger().with_mut(|li| {
        li.timestamp += 60 * 60 * 24 * 365;
    });
    client.expire_trial_offers(&admin, &player);

    // Purge now succeeds.
    client.purge_player_data(&admin, &player);

    // Trial offers and escrows are gone.
    assert_eq!(client.get_trial_count(&player), 0);
    assert!(client.get_trial_offers(&player).is_empty());
    assert!(client.get_trial_escrows(&player).is_empty());
}

#[test]
fn purge_clears_player_contacts() {
    let (env, client, admin, player, _token) = setup();

    // Seed a contact so we can observe it being removed.
    let contact = PlayerContact {
        name: Symbol::new(&env, "agent"),
        address: Address::generate(&env),
    };
    client.add_player_contact(&admin, &player, &contact);
    assert!(!client.get_player_contacts(&player).is_empty());

    client.purge_player_data(&admin, &player);

    assert!(client.get_player_contacts(&player).is_empty());
}

#[test]
fn purge_requires_admin_auth() {
    let (env, client, admin, player, _token) = setup();

    let non_admin = Address::generate(&env);

    // Scope auth to the non-admin caller only; the contract must reject them.
    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "purge_player_data",
            args: (non_admin.clone(), player.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_purge_player_data(&non_admin, &player);
    assert!(result.is_err());
}

#[test]
fn purge_refused_when_contract_paused() {
    let (_env, client, admin, player, _token) = setup();

    client.pause(&admin);

    // Current behaviour: a paused contract returns `ContractPaused`.
    let result = client.try_purge_player_data(&admin, &player);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

#[test]
fn purge_emits_event_with_entries_removed() {
    let (env, client, admin, player, _token) = setup();
    create_trial_offers(&env, &client, &admin, &player, 2);

    // Add a contact so entries_removed is non-trivial.
    let contact = PlayerContact {
        name: Symbol::new(&env, "agent"),
        address: Address::generate(&env),
    };
    client.add_player_contact(&admin, &player, &contact);

    client.purge_player_data(&admin, &player);

    // Find the purge event and check its `entries_removed` payload.
    let events = env.events().all();
    let mut found = false;
    for (_contract, topics, data) in events.iter() {
        // Topics: (symbol "purge_player_data", player)
        if topics.len() >= 1 {
            let topic0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            if let Ok(sym) = topic0 {
                if sym == Symbol::new(&env, "purge_player_data") {
                    let removed = u32::try_from_val(&env, &data).unwrap();
                    // 2 trial offers + 2 escrows + 1 contact.
                    assert_eq!(removed, 5);
                    found = true;
                }
            }
        }
    }
    assert!(found, "purge_player_data event was not emitted");
}