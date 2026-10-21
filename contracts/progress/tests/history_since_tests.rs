//! Tests for `get_history_since` — issue #1702.
//!
//! History entries are produced with admin `reset_player_level`, alternating
//! between two levels, with the ledger timestamp set to `entry_index * 100`.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String, Vec};

struct Harness {
    env: Env,
    client: ProgressContractClient<'static>,
    player_id: u64,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    let admin = Address::generate(&env);

    let reg_id = env.register(RegistrationContract, ());
    let registration = RegistrationContractClient::new(&env, &reg_id);
    registration.initialize(&admin);

    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&admin);
    client.set_registration_contract(&reg_id);
    registration.set_progress_contract(&id);

    let vitals = PlayerVitals {
        age: 20,
        position: String::from_str(&env, "ST"),
        region: String::from_str(&env, "EU"),
        nationality: String::from_str(&env, "FR"),
    };
    let hashes = Vec::from_array(&env, [String::from_str(&env, "bafytestcid")]);
    let player_id = registration.register_player(&Address::generate(&env), &vitals, &hashes);

    Harness {
        env,
        client,
        player_id,
    }
}

/// Record `n` history entries; entry `i` (1-based) has `updated_at == i * 100`.
fn record_n(h: &Harness, n: u64) {
    for i in 1..=n {
        h.env.ledger().set_timestamp(i * 100);
        let target = if i % 2 == 1 {
            ProgressLevel::VerifiedIdentity
        } else {
            ProgressLevel::Unverified
        };
        h.client.reset_player_level(&h.player_id, &target);
    }
}

fn timestamps(entries: &Vec<scoutchain_progress::ProgressEntry>) -> std::vec::Vec<u64> {
    entries.iter().map(|e| e.updated_at).collect()
}

#[test]
fn test_no_history_returns_empty() {
    let h = setup();
    assert_eq!(h.client.get_history_since(&h.player_id, &0, &10).len(), 0);
}

#[test]
fn test_future_since_timestamp_returns_empty() {
    let h = setup();
    record_n(&h, 5);
    assert_eq!(h.client.get_history_since(&h.player_id, &10_000, &10).len(), 0);
}

#[test]
fn test_since_zero_returns_newest_limit_entries_newest_first() {
    let h = setup();
    record_n(&h, 5);
    let res = h.client.get_history_since(&h.player_id, &0, &3);
    assert_eq!(timestamps(&res), [500, 400, 300]);
}

#[test]
fn test_entries_spanning_page_boundary() {
    let h = setup();
    record_n(&h, 10); // page 0: entries 1..=8, page 1: entries 9..=10
    let res = h.client.get_history_since(&h.player_id, &300, &50);
    assert_eq!(timestamps(&res), [1000, 900, 800, 700, 600, 500, 400, 300]);
}

#[test]
fn test_limit_zero_behaves_like_one() {
    let h = setup();
    record_n(&h, 3);
    let res = h.client.get_history_since(&h.player_id, &0, &0);
    assert_eq!(timestamps(&res), [300]);
}

/// Pins current behaviour: with 90 matching entries the result is capped by
/// the 50-entry limit clamp (before the 10-page / 80-entry scan bound kicks in)
/// and contains the newest 50 entries, newest-first.
#[test]
fn test_more_than_80_matching_entries_truncates_to_newest_50() {
    let h = setup();
    record_n(&h, 90);
    let res = h.client.get_history_since(&h.player_id, &0, &1000);
    let expected: std::vec::Vec<u64> = (41..=90).rev().map(|i| i * 100).collect();
    assert_eq!(timestamps(&res), expected);
}
