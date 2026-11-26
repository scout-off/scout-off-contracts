I need to see the existing code to write accurate tests. Let me look at the relevant files.

Since I don't have the actual file contents provided, I'll need to infer the structure from the issue description and common patterns. However, I cannot write correct tests without knowing the actual API. Let me provide the test file based on the described requirements and typical Soroban patterns.

```rust
//! Tests for `registration::restore_player_record`.
//!
//! Mirrors the structure of the `restore_scout_record` tests.

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, IntoVal, Symbol, TryFromVal,
};

use registration::{errors::Error, events::PlayerRecordRestored, RegistrationContract, RegistrationContractClient};

mod setup;

use setup::{register_contract, setup_env};

/// Helper: register a player and return the player id.
fn register_player(
    env: &Env,
    client: &RegistrationContractClient,
    admin: &Address,
    player: &Address,
) -> u64 {
    client.mock_all_auths();
    client.register_player(admin, player)
}

#[test]
fn restore_player_record_extends_ttl_for_existing_player() {
    let (env, admin, client) = setup_env();
    let player = Address::generate(&env);

    let player_id = register_player(&env, &client, &admin, &player);

    // Advance the ledger so the TTL is meaningfully extended.
    env.ledger().with_mut(|l| {
        l.sequence_number += 100;
    });

    let ttl_before = env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .get_ttl(&registration::storage::DataKey::Player(player_id))
    });

    client.mock_all_auths();
    client.restore_player_record(&admin, &player_id);

    let ttl_after = env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .get_ttl(&registration::storage::DataKey::Player(player_id))
    });

    assert!(
        ttl_after > ttl_before,
        "expected Player({player_id}) TTL to be extended: before={ttl_before}, after={ttl_after}"
    );
}

#[test]
fn restore_player_record_unknown_player_returns_player_not_found() {
    let (env, admin, client) = setup_env();
    let unknown_id: u64 = 999_999;

    client.mock_all_auths();
    let result = client.try_restore_player_record(&admin, &unknown_id);

    assert_eq!(
        result,
        Err(Ok(Error::PlayerNotFound)),
        "expected PlayerNotFound for unknown player id"
    );
}

#[test]
fn restore_player_record_non_admin_fails_auth() {
    let (env, admin, client) = setup_env();
    let player = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let player_id = register_player(&env, &client, &admin, &player);

    // Do NOT mock auths — the non-admin caller must fail authorization.
    let result = client.try_restore_player_record(&non_admin, &player_id);

    assert!(
        result.is_err(),
        "expected non-admin caller to fail authorization"
    );
}

#[test]
fn restore_player_record_emits_event() {
    let (env, admin, client) = setup_env();
    let player = Address::generate(&env);

    let player_id = register_player(&env, &client, &admin, &player);

    client.mock_all_auths();
    client.restore_player_record(&admin, &player_id);

    let expected = PlayerRecordRestored {
        admin: admin.clone(),
        player_id,
    };

    let events = env.events().all();
    let found = events.iter().any(|(contract, topics, data)| {
        contract == client.address
            && topics
                == soroban_sdk::vec![
                    &env,
                    Symbol::new(&env, "player_record_restored").into_val(&env),
                ]
            && PlayerRecordRestored::try_from_val(&env, &data).map(|e| e == expected).unwrap_or(false)
    });

    assert!(found, "expected PlayerRecordRestored event to be emitted");
}

#[test]
fn restore_player_record_extends_index_ttls() {
    let (env, admin, client) = setup_env();
    let player = Address::generate(&env);

    let player_id = register_player(&env, &client, &admin, &player);

    env.ledger().with_mut(|l| {
        l.sequence_number += 100;
    });

    // The doc comment for `restore_player_record` claims it also extends the
    // TTL of the player index entry. Assert that here.
    let index_key = registration::storage::DataKey::PlayerIndex(player_id);
    let ttl_before = env.as_contract(&client.address, || {
        env.storage().persistent().get_ttl(&index_key)
    });

    client.mock_all_auths();
    client.restore_player_record(&admin, &player_id);

    let ttl_after = env.as_contract(&client.address, || {
        env.storage().persistent().get_ttl(&index_key)
    });

    assert!(
        ttl_after > ttl_before,
        "expected PlayerIndex({player_id}) TTL to be extended: before={ttl_before}, after={ttl_after}"
    );
}
```

**Important caveat:** I was unable to see the actual source files (`registration` crate, `setup` module, `storage::DataKey` variants, `events::PlayerRecordRestored` definition, and the `restore_player_record` signature). The code above is written against the most likely API surface based on the issue description and standard Soroban conventions, but several identifiers must be verified against the real codebase:

- `registration::storage::DataKey::Player(id)` and `PlayerIndex(id)` — exact variant names.
- `registration::errors::Error::PlayerNotFound` — exact error path.
- `registration::events::PlayerRecordRestored` — exact struct name and fields.
- `client.restore_player_record(&admin, &player_id)` — exact signature/argument order.
- `setup::{setup_env, register_contract}` — exact helper names and return tuple.
- `client.register_player(...)` — exact registration helper signature.

Please share the actual contents of `contracts/registration/src/lib.rs`, `contracts/registration/src/storage.rs`, `contracts/registration/src/events.rs`, `contracts/registration/src/errors.rs`, `contracts/registration/tests/setup.rs`, and `contracts/registration/tests/restore_scout_record.rs` so I can produce a file that compiles against the real API.