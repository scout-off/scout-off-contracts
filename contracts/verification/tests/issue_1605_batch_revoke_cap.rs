//! Tests for issue #1605: `batch_revoke_validators` must be bounded and must
//! defer the for-cause cascade sweep to `continue_revocation_cascade`.

use scoutchain_verification::{
    AttestationStatus, RevocationSeverity, VerificationContract, VerificationContractClient,
    VerificationError,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

// ── helpers ──────────────────────────────────────────────────────────────────

/// All CIDv0-valid characters (no 0/O/I/l).
const BASE58_CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Deterministically build a distinct valid 46-char CIDv0 for a given seed.
fn cid(env: &Env, seed: u32) -> String {
    let mut s = std::string::String::from("Qm");
    let mut n = seed.wrapping_add(1);
    for _ in 0..44 {
        let idx = (n % BASE58_CHARS.len() as u32) as usize;
        s.push(BASE58_CHARS[idx] as char);
        n = n / (BASE58_CHARS.len() as u32) + seed.wrapping_add(7);
    }
    String::from_str(env, &s)
}

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn register_validator(env: &Env, client: &VerificationContractClient) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, "UEFA-B-License-2026"),
        &String::from_str(env, "Default Academy"),
        &Vec::new(env),
    );
    wallet
}

/// Approve `count` distinct milestones for `validator` across `count` distinct
/// player IDs, using deterministic CIDs starting from `cid_seed`.
///
/// Returns the next cid_seed to use for subsequent calls.
fn approve_n_milestones(
    env: &Env,
    client: &VerificationContractClient,
    validator: &Address,
    count: u32,
    cid_seed_start: u32,
) -> u32 {
    for i in 0..count {
        let player_id = (cid_seed_start + i + 1) as u64;
        let c = cid(env, cid_seed_start + i);
        client.approve_milestone(
            validator,
            &player_id,
            &String::from_str(env, "verified achievement"),
            &c,
            &None,
        );
    }
    cid_seed_start + count
}

fn register_validator_n(env: &Env, client: &VerificationContractClient, n: u32) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, &format!("UEFA-B-License-{n}")),
        &String::from_str(env, "Default Academy"),
        &Vec::new(env),
    );
    wallet
}

#[test]
fn oversized_batch_is_rejected_before_any_state_change() {
    let (env, client) = setup();
    let mut wallets = Vec::new(&env);
    for _ in 0..11 {
        wallets.push_back(register_validator_n(&env, &client, wallets.len()));
    }
    let before = client.get_active_validator_count();
    let res = client.try_batch_revoke_validators(&wallets, &RevocationSeverity::ForCause, &None);
    assert_eq!(res, Err(Ok(VerificationError::BatchTooLarge)));
    assert_eq!(client.get_active_validator_count(), before);
    assert!(client.get_validator(&wallets.get(0).unwrap()).active);
}

#[test]
fn for_cause_batch_deactivates_immediately_and_defers_cascade() {
    let (env, client) = setup();
    let v1 = register_validator_n(&env, &client, 0);
    let v2 = register_validator_n(&env, &client, 1);
    // 60 > CASCADE_LIMIT so each cascade needs a continuation call.
    approve_n_milestones(&env, &client, &v1, 60, 0);
    approve_n_milestones(&env, &client, &v2, 60, 1000);

    let mut wallets = Vec::new(&env);
    wallets.push_back(v1.clone());
    wallets.push_back(v2.clone());
    client.batch_revoke_validators(&wallets, &RevocationSeverity::ForCause, &None);

    // Validators are inactive straight away; no milestone is flagged yet.
    assert!(!client.get_validator(&v1).active);
    assert!(!client.get_validator(&v2).active);
    assert!(!client.is_milestone_flagged(&1u64, &1u32));
    assert!(!client.is_milestone_flagged(&1001u64, &1u32));

    // Drive both cascades to completion.
    for _ in 0..2 {
        client.continue_revocation_cascade(&v1);
        client.continue_revocation_cascade(&v2);
    }
    for i in 1u64..=60 {
        assert!(client.is_milestone_flagged(&i, &1u32), "v1 milestone {i}");
        assert!(
            client.is_milestone_flagged(&(1000 + i), &1u32),
            "v2 milestone {i}"
        );
    }
}

#[test]
fn max_size_batch_succeeds() {
    let (env, client) = setup();
    let mut wallets = Vec::new(&env);
    for i in 0..10u32 {
        wallets.push_back(register_validator_n(&env, &client, i));
    }
    client.batch_revoke_validators(&wallets, &RevocationSeverity::ForCause, &None);
    assert_eq!(client.get_active_validator_count(), 0);
}
