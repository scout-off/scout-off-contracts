//! Tests for the player-data purge / erasure feature on the verification
//! contract (issue #1781).
//!
//! Covers:
//! - `get_purge_status` zero-value before any purge
//! - `purge_player_data` for a player with no milestones (page 0 completes)
//! - `purge_player_data` for a player with milestones across multiple pages
//! - Resumability across calls
//! - Wrong cursor is rejected
//! - Idempotency after completion

use scoutchain_verification::{PurgeStatus, VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

struct Harness {
    env: Env,
    client: VerificationContractClient<'static>,
    admin: Address,
    validator: Address,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let validator = Address::generate(&env);
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    client.initialize(&admin);
    client.register_validator(
        &validator,
        &String::from_str(&env, "UEFA B License 2024"),
        &String::from_str(&env, "Test Academy"),
        &Vec::new(&env),
    );
    Harness {
        env,
        client,
        admin,
        validator,
    }
}

// A pool of valid CIDv0 strings (each is exactly 46 chars, starts with "Qm",
// base58btc characters only). Using a fixed pool avoids character-mapping bugs.
const CIDS: &[&str] = &[
    "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB",
    "QmW2WQi7j6c7UgJTarActp7tDNikE4B2qXtFCfLPdsgaTQ",
    "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG",
    "QmSoLnSGccFuZQJzRadHn95W2CrSFackH1DrAoFEv5TPBY",
    "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    "QmSoLSafTMBsPKadTEgaXctDQVcqN88CNLHXMkTNwMKPnu",
    "QmSoLV4Bbm51jM9C4gDYZQ9Cy3U6aXMJDAbzgu2fzaDs64",
    "QmSoLer265NRgSp2LA3dPaeykiS1J6DifTC88f5uVQKNAd",
    "QmbQBUSRL9raZtNXfpTDeaxALDRSe9P9KmFqUy8bXBqGpA",
    "QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
    "QmR7tiySn6vFHcEjBeZNtYGAFh735PJHfRJ5bYZkZbQFBr",
    "QmUNLLsPACCz1vLxQVkXqqLX5R1X345qqfHbsf67hvA3Nn",
    "QmYbcW4tXLXHWw753boCK8Y7uxLu5abXjyYizhLznq9PkW",
    "QmPChd2hVbrJ6bfo3WBcTW4iZnpHm8TEzWkLHmLpXhF68A",
    "QmSUhPvNEBiTBzMvFcq2DvQWetjFZmFEPCPMnL7HMNRh9b",
    "QmRgutAxd8t7oGkSm4wmeuByG6M51wcTso6cubDdQtuEfL",
    "QmZU8bKEG6fmgmenJoPfLED8tgybovdN9PUTZEQU9XFN5b",
    "QmTHd1LGCQ2eHhx3SLmpzMHtDHGm1LV5WwYmRm4fdFMVaW",
    "QmaozNR7DZHQK1ZcU9p7QdrshMvXqWK6gpu5rmrkPdT3L4",
    "QmbNgNPPykP4YTuAeSa3DsnBJWLVxccrqLUZDpNtzDCeKB",
];

/// Approve `n` distinct milestones for `player_id` using pre-validated CIDv0 hashes.
/// `base_index` offsets into the CID pool so different (player, milestone) pairs
/// use different hashes.
fn approve_n(h: &Harness, player_id: u64, n: u32) {
    let base = ((player_id as usize) * 5) % CIDS.len();
    for i in 0..n as usize {
        let cid = CIDS[(base + i) % CIDS.len()];
        h.client.approve_milestone(
            &h.validator,
            &player_id,
            &String::from_str(&h.env, &format!("Milestone {} p{}", i + 1, player_id)),
            &String::from_str(&h.env, cid),
            &None,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_purge_status_before_start_is_zero() {
    let h = setup();
    let status = h.client.get_purge_status(&99u64);
    assert_eq!(
        status,
        PurgeStatus {
            started: false,
            cursor: 0,
            completed: false
        }
    );
}

#[test]
fn test_purge_no_milestones_completes_on_first_call() {
    let h = setup();
    let pid: u64 = 1;
    let (nc, more) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(nc, 0u32);
    assert!(!more);
    let s = h.client.get_purge_status(&pid);
    assert!(s.started);
    assert!(s.completed);
}

#[test]
fn test_purge_with_milestones_lifecycle() {
    let h = setup();
    let pid: u64 = 2;
    approve_n(&h, pid, 2);

    // Page 0: scalar keys removed; more=true because milestones exist.
    let (c1, more1) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert!(more1);
    assert_eq!(c1, 1u32);

    let mid = h.client.get_purge_status(&pid);
    assert!(mid.started);
    assert!(!mid.completed);

    // Page 1: milestone records removed; done.
    let (c2, more2) = h.client.purge_player_data(&h.admin, &pid, &c1);
    assert!(!more2);
    assert_eq!(c2, 0u32);

    let done = h.client.get_purge_status(&pid);
    assert!(done.started);
    assert!(done.completed);
}

#[test]
fn test_purge_wrong_cursor_rejected() {
    use scoutchain_verification::VerificationError;
    let h = setup();
    let pid: u64 = 3;
    approve_n(&h, pid, 1);

    // Advance stored cursor to 1.
    h.client.purge_player_data(&h.admin, &pid, &0u32);

    // Supplying 0 again must fail.
    let result = h.client.try_purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(result, Err(Ok(VerificationError::InvalidInput)));
}

#[test]
fn test_purge_idempotent_after_completion() {
    let h = setup();
    let pid: u64 = 4;
    h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert!(h.client.get_purge_status(&pid).completed);

    let (nc, more) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(nc, 0u32);
    assert!(!more);
}

#[test]
fn test_purge_two_players_independent() {
    let h = setup();
    approve_n(&h, 10, 1);
    approve_n(&h, 11, 1);

    // Fully purge player 10.
    let (c, more) = h.client.purge_player_data(&h.admin, &10u64, &0u32);
    if more {
        h.client.purge_player_data(&h.admin, &10u64, &c);
    }
    assert!(h.client.get_purge_status(&10u64).completed);

    // Player 11 untouched.
    assert!(!h.client.get_purge_status(&11u64).started);
}
