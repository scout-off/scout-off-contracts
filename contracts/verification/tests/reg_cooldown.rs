//! Per-wallet registration cooldown tests for the verification contract.
//!
//! `register_validator` (and each entry in `batch_register_validators`) must
//! reject a re-registration of the same wallet while its cooldown window
//! (`ValidatorRegLastSent` + `RegCooldownSecs`) has not elapsed, returning
//! `RegistrationCooldown` (code 25).

use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{
    testutils::Address as _, vec, Address, Env, String, Vec,
};

const CREDENTIALS: &str = "UEFA-B-License-2026";
const AFFILIATION: &str = "Default Academy";
const START: u64 = 1_000_000;
const COOLDOWN_SECS: u64 = 3_600; // 1 hour

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    // Enable the cooldown (default is 0 = disabled).
    client.set_reg_cooldown(&COOLDOWN_SECS);
    (env, client)
}

fn entry(env: &Env, wallet: Address) -> (Address, String, String, Vec<String>) {
    (
        wallet,
        String::from_str(env, CREDENTIALS),
        String::from_str(env, AFFILIATION),
        vec![env],
    )
}

/// A wallet that registers again within its cooldown window is rejected with
/// `RegistrationCooldown` (code 25).
#[test]
fn test_register_validator_respects_cooldown() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    client.register_validator(
        &wallet,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, AFFILIATION),
        &vec![&env],
    );

    // Immediately re-register the same wallet -> within the cooldown window.
    let result = client.try_register_validator(
        &wallet,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, AFFILIATION),
        &vec![&env],
    );
    assert!(
        matches!(
            result,
            Err(Ok(scoutchain_verification::VerificationError::RegistrationCooldown))
        ),
        "re-registration within cooldown must return RegistrationCooldown: {result:?}"
    );

    // A different wallet is unaffected by wallet A's cooldown.
    let other = Address::generate(&env);
    client.register_validator(
        &other,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, AFFILIATION),
        &vec![&env],
    );
}

/// After the cooldown window elapses, re-registering the wallet moves past the
/// cooldown check (and is then rejected as an existing registration).
#[test]
fn test_register_validator_cooldown_expires() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    client.register_validator(
        &wallet,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, AFFILIATION),
        &vec![&env],
    );

    // Advance past the cooldown window.
    env.ledger().with_mut(|l| l.timestamp = START + COOLDOWN_SECS + 1);

    let result = client.try_register_validator(
        &wallet,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, AFFILIATION),
        &vec![&env],
    );
    assert!(
        matches!(
            result,
            Err(Ok(scoutchain_verification::VerificationError::ValidatorAlreadyRegistered))
        ),
        "after cooldown the wallet is detected as already registered: {result:?}"
    );
}

/// A batch entry whose wallet is inside its cooldown window fails the whole
/// batch with `RegistrationCooldown` and persists no state.
#[test]
fn test_batch_register_validators_respects_cooldown() {
    let (env, client) = setup();
    let before = Address::generate(&env);
    client.register_validator(
        &before,
        &String::from_str(&env, CREDENTIALS),
        &String::from_str(&env, AFFILIATION),
        &vec![&env],
    );

    // `before` is within its cooldown; the batch must be rejected atomically.
    let fresh = Address::generate(&env);
    let entries = vec![
        &env,
        entry(&env, before.clone()),
        entry(&env, fresh.clone()),
    ];

    let result = client.try_batch_register_validators(&entries);
    assert!(
        matches!(
            result,
            Err(Ok(scoutchain_verification::VerificationError::RegistrationCooldown))
        ),
        "batch with a cooldown-locked wallet must return RegistrationCooldown: {result:?}"
    );

    // Nothing was persisted: `fresh` was not registered by the failed batch.
    assert_eq!(
        client.get_active_validator_count(),
        1,
        "failed batch must not register any validators"
    );
}

/// batch_register_validators succeeds for fresh wallets when the cooldown is
/// enabled and no wallet in the batch has a pending cooldown.
#[test]
fn test_batch_register_validators_succeeds_with_cooldown_enabled() {
    let (env, client) = setup();
    let w1 = Address::generate(&env);
    let w2 = Address::generate(&env);
    let entries = vec![&env, entry(&env, w1.clone()), entry(&env, w2.clone())];

    client.batch_register_validators(&entries);
    assert_eq!(client.get_active_validator_count(), 2);
}