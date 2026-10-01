//! Regression tests for issue #1390: `batch_revoke_validators` never
//! decremented `ActiveValidatorCount` and re-revoked already-revoked wallets.
//!
//! Before the fix the batch path set `active = false` but skipped the counter
//! decrement that the single-wallet `revoke_validator` path performs, so
//! `get_active_validator_count()` over-reported after any batch revocation.
//! Both entrypoints now share the `revoke_one` helper, the batch path rewrites
//! `ValidatorVector` once instead of once per wallet, and a batch that lists
//! the same wallet twice is rejected up front so the resulting count is exact.

use scoutchain_verification::{
    DataKey, RevocationSeverity, VerificationContract, VerificationContractClient,
    VerificationError,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String, Vec};

/// Register a validator and return its address.
fn register(client: &VerificationContractClient, env: &Env, cred: &str) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, cred),
        &String::from_str(env, "Default Academy"),
        &Vec::new(env),
    );
    wallet
}

/// Build a fresh env with `n` registered, active validators.
fn setup(n: usize) -> (Env, Address, Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths();

    let verification_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &verification_id);
    client.initialize(&Address::generate(&env));

    let mut wallets = Vec::new(&env);
    for i in 0..n {
        let cred = format!("License-2026-{}", i);
        wallets.push_back(register(&client, &env, &cred));
    }

    (env, verification_id, wallets)
}

#[test]
fn batch_revoke_decrements_active_validator_count() {
    let (env, _id, wallets) = setup(3);
    let client = VerificationContractClient::new(&env, &_id);

    assert_eq!(client.get_active_validator_count(), 3);

    client.batch_revoke_validators(
        &vec![&env, wallets.get(0).unwrap(), wallets.get(1).unwrap()],
        &RevocationSeverity::Routine,
        &None,
    );

    // Two active validators were revoked, so the count must drop by exactly
    // two. This is the core #1390 regression: the old batch path left it at 3.
    assert_eq!(
        client.get_active_validator_count(),
        1,
        "batch_revoke_validators must decrement ActiveValidatorCount once per \
         revoked active validator"
    );
}

#[test]
fn batch_revoke_skips_decrement_for_already_inactive_validator() {
    let (env, id, wallets) = setup(3);
    let client = VerificationContractClient::new(&env, &id);

    // Revoke one wallet through the single-wallet path first, so it is already
    // inactive when the batch runs.
    client.revoke_validator(
        &wallets.get(0).unwrap(),
        &RevocationSeverity::Routine,
        &None,
    );
    assert_eq!(client.get_active_validator_count(), 2);

    // Batch contains the already-inactive wallet plus one active wallet.
    client.batch_revoke_validators(
        &vec![&env, wallets.get(0).unwrap(), wallets.get(1).unwrap()],
        &RevocationSeverity::Routine,
        &None,
    );

    // Only the genuinely active wallet may decrement the counter. Decrementing
    // twice would underflow the count relative to the real active set.
    assert_eq!(
        client.get_active_validator_count(),
        1,
        "re-revoking an already-inactive validator must not decrement the count again"
    );

    // The counter must still agree with the actual number of active validators.
    let active_on_chain: u32 = env.as_contract(&id, || {
        let mut n = 0u32;
        for i in 0..3u32 {
            let w = env
                .storage()
                .persistent()
                .get::<DataKey, scoutchain_verification::Validator>(&DataKey::Validator(
                    wallets.get(i).unwrap(),
                ))
                .unwrap();
            if w.active {
                n += 1;
            }
        }
        n
    });
    assert_eq!(active_on_chain, 1);
    assert_eq!(client.get_active_validator_count(), active_on_chain);
}

#[test]
fn batch_revoke_rejects_duplicate_wallets() {
    let (env, id, wallets) = setup(3);
    let client = VerificationContractClient::new(&env, &id);

    let result = client.try_batch_revoke_validators(
        &vec![
            &env,
            wallets.get(0).unwrap(),
            wallets.get(1).unwrap(),
            wallets.get(0).unwrap(),
        ],
        &RevocationSeverity::Routine,
        &None,
    );

    assert_eq!(
        result,
        Err(Ok(VerificationError::InvalidInput)),
        "a batch listing the same wallet twice must be rejected with InvalidInput"
    );

    // The rejection must be atomic: nothing revoked, count untouched.
    assert_eq!(
        client.get_active_validator_count(),
        3,
        "a rejected duplicate-wallet batch must not change the active count"
    );
}

#[test]
fn batch_revoke_updates_validator_vector_for_every_wallet() {
    let (env, id, wallets) = setup(3);
    let client = VerificationContractClient::new(&env, &id);

    client.batch_revoke_validators(
        &vec![&env, wallets.get(0).unwrap(), wallets.get(1).unwrap()],
        &RevocationSeverity::Routine,
        &None,
    );

    // Vector is rewritten once for the whole batch; both revoked wallets must
    // be gone and the surviving one must remain, in its original order.
    let vector: Vec<Address> = env.as_contract(&id, || {
        env.storage()
            .persistent()
            .get(&DataKey::ValidatorVector)
            .unwrap_or_else(|| Vec::new(&env))
    });

    assert_eq!(vector.len(), 1, "both revoked wallets must be removed");
    assert_eq!(
        vector.get(0).unwrap(),
        wallets.get(2).unwrap(),
        "the surviving validator must remain in the vector"
    );
}

#[test]
fn batch_revoke_rejects_unknown_wallet_atomically() {
    let (env, id, wallets) = setup(2);
    let client = VerificationContractClient::new(&env, &id);
    let stranger = Address::generate(&env);

    let result = client.try_batch_revoke_validators(
        &vec![&env, wallets.get(0).unwrap(), stranger],
        &RevocationSeverity::Routine,
        &None,
    );

    assert_eq!(result, Err(Ok(VerificationError::ValidatorNotFound)));
    assert_eq!(
        client.get_active_validator_count(),
        2,
        "a batch containing an unknown wallet must not partially revoke"
    );
}
