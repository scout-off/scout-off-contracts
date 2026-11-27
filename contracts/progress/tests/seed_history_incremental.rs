//! Issue #1618: `admin_seed_history` maintains the Merkle root incrementally
//! (O(log n) hashes per call) and writes the paged history layout.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressEntry};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, IntoVal, TryFromVal, Val, Vec,
};

/// Budget for seeding the 500th history entry (see `ci/cpu-cost-budget.md`).
const SEED_500TH_ENTRY_CPU_BUDGET: u64 = 15_000_000;

fn setup() -> (Env, ProgressContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(ProgressContract, (Address::generate(&env),));
    let client = ProgressContractClient::new(&env, &id);
    client.open_migration_window();
    (env, client)
}

/// Entries alternate an advance (Unverified -> VerifiedIdentity) with a reset
/// back to Unverified, so every seeded history forms a valid level chain
/// (#1617: `old_level` must continue the previous entry's `new_level`).
fn make_entry(player_id: u64, index: u32, by: &Address) -> ProgressEntry {
    let advance = index % 2 == 1;
    ProgressEntry {
        player_id,
        old_level: if advance {
            ProgressLevel::Unverified
        } else {
            ProgressLevel::VerifiedIdentity
        },
        new_level: if advance {
            ProgressLevel::VerifiedIdentity
        } else {
            ProgressLevel::Unverified
        },
        updated_by: by.clone(),
        updated_at: 1_700_000_000 + index as u64 * 100,
        milestone_ref: if advance { index } else { 0 },
        ledger_sequence: 1000 + index,
    }
}

fn level_code(l: &ProgressLevel) -> u32 {
    match l {
        ProgressLevel::Unverified => 0,
        ProgressLevel::VerifiedIdentity => 1,
        ProgressLevel::PerformanceMilestones => 2,
        ProgressLevel::EliteTier => 3,
    }
}

/// Independent RFC 6962 leaf hash: `H(0x00 || canonical entry bytes)`.
fn leaf(env: &Env, e: &ProgressEntry) -> BytesN<32> {
    let mut b = Bytes::new(env);
    b.push_back(0u8);
    b.extend_from_slice(&e.player_id.to_be_bytes());
    b.extend_from_slice(&level_code(&e.old_level).to_be_bytes());
    b.extend_from_slice(&level_code(&e.new_level).to_be_bytes());
    b.append(&e.updated_by.clone().to_xdr(env));
    b.extend_from_slice(&e.updated_at.to_be_bytes());
    b.extend_from_slice(&e.milestone_ref.to_be_bytes());
    b.extend_from_slice(&e.ledger_sequence.to_be_bytes());
    env.crypto().sha256(&b).to_bytes()
}

/// Independent recursive RFC 6962 Merkle Tree Hash over `leaves[start..end]`.
fn mth_range(env: &Env, leaves: &Vec<BytesN<32>>, start: u32, end: u32) -> BytesN<32> {
    let n = end - start;
    if n == 1 {
        return leaves.get(start).unwrap();
    }
    let mut k = 1u32;
    while k * 2 < n {
        k *= 2;
    }
    let left = mth_range(env, leaves, start, start + k);
    let right = mth_range(env, leaves, start + k, end);
    let mut b = Bytes::new(env);
    b.push_back(1u8);
    b.extend_from_slice(&left.to_array());
    b.extend_from_slice(&right.to_array());
    env.crypto().sha256(&b).to_bytes()
}

#[test]
fn test_seed_root_matches_independent_mth_for_1_to_256_entries() {
    let (env, client) = setup();
    let player_id = 7u64;
    let by = Address::generate(&env);
    let mut leaves: Vec<BytesN<32>> = Vec::new(&env);

    for i in 1..=256u32 {
        let entry = make_entry(player_id, i, &by);
        leaves.push_back(leaf(&env, &entry));
        client.admin_seed_history(&player_id, &i, &entry, &None);

        let expected = mth_range(&env, &leaves, 0, i);
        assert_eq!(
            client.get_progress_root(&player_id),
            expected,
            "root mismatch after seeding {i} entries"
        );
    }
    assert_eq!(client.get_history_count(&player_id), 256);
}

#[test]
fn test_seed_500th_entry_cpu_budget() {
    let (env, client) = setup();
    let player_id = 9u64;
    let by = Address::generate(&env);

    env.cost_estimate().budget().reset_unlimited();
    for i in 1..500u32 {
        client.admin_seed_history(&player_id, &i, &make_entry(player_id, i, &by), &None);
    }

    env.cost_estimate().budget().reset_default();
    client.admin_seed_history(&player_id, &500u32, &make_entry(player_id, 500, &by), &None);
    let cost = env.cost_estimate().budget().cpu_instruction_cost();
    assert!(
        cost <= SEED_500TH_ENTRY_CPU_BUDGET,
        "CPU budget exceeded for `admin_seed_history` (500th entry): measured {cost} instructions (budget {SEED_500TH_ENTRY_CPU_BUDGET})"
    );
}

/// Decode the `entries_removed` / `more` payload of a `purge_player_data` event.
fn purge_event_payload(env: &Env, event: &soroban_sdk::testutils::ContractEvents) -> (u32, bool) {
    let topics = event.1.clone();
    assert_eq!(topics.len(), 2, "purge event should have 2 topics");
    let topic: soroban_sdk::Symbol =
        soroban_sdk::Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
    assert_eq!(topic, soroban_sdk::Symbol::new(env, "purge_player_data"));

    let data: Val = event.2.clone();
    let tuple = <(u32, bool)>::try_from_val(env, &data).unwrap();
    tuple
}

/// Collect the `(entries_removed, more)` payloads of all purge events, in order.
fn purge_events(env: &Env) -> Vec<(u32, bool)> {
    let mut out = Vec::new(env);
    for event in env.events().all().iter() {
        let topics = event.1.clone();
        if topics.len() != 2 {
            continue;
        }
        let topic: soroban_sdk::Symbol =
            soroban_sdk::Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if topic == soroban_sdk::Symbol::new(env, "purge_player_data") {
            out.push_back(<(u32, bool)>::try_from_val(env, &event.2).unwrap());
        }
    }
    out
}

#[test]
fn test_purge_player_data_no_history() {
    let (env, client) = setup();
    let player_id = 42u64;

    let (removed, more) = client.purge_player_data(&player_id, &None);
    assert_eq!(removed, 0);
    assert!(!more);

    assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(client.get_history_count(&player_id), 0);

    let events = purge_events(&env);
    assert_eq!(events.len(), 1);
    assert_eq!(events.get(0).unwrap(), (0u32, false));
}

#[test]
fn test_purge_player_data_three_entries_single_call() {
    let (env, client) = setup();
    let player_id = 11u64;
    let by = Address::generate(&env);

    for i in 1..=3u32 {
        client.admin_seed_history(&player_id, &i, &make_entry(player_id, i, &by), &None);
    }
    assert_eq!(client.get_history_count(&player_id), 3);

    let (removed, more) = client.purge_player_data(&player_id, &None);
    assert_eq!(removed, 3);
    assert!(!more);

    assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(client.get_history_count(&player_id), 0);
    assert_eq!(
        client.get_progress_root(&player_id),
        BytesN::from_array(&env, &[0u8; 32])
    );

    let events = purge_events(&env);
    assert_eq!(events.len(), 1);
    assert_eq!(events.get(0).unwrap(), (3u32, false));
}

#[test]
fn test_purge_player_data_pages_through_120_entries() {
    let (env, client) = setup();
    let player_id = 23u64;
    let by = Address::generate(&env);

    for i in 1..=120u32 {
        client.admin_seed_history(&player_id, &i, &make_entry(player_id, i, &by), &None);
    }
    assert_eq!(client.get_history_count(&player_id), 120);

    let mut cursor: Option<u32> = None;
    let mut total_removed = 0u32;
    let mut calls = 0u32;
    let mut last_cursor: Option<u32> = None;

    loop {
        let (removed, more) = client.purge_player_data(&player_id, &cursor);
        calls += 1;
        total_removed += removed;
        assert!(removed > 0, "each page should remove at least one entry");

        if more {
            // Cursor must advance monotonically.
            let next = cursor.map(|c| c + removed).unwrap_or(removed);
            if let Some(prev) = last_cursor {
                assert!(next > prev, "next_cursor must be monotonic");
            }
            last_cursor = Some(next);
            cursor = Some(next);
        } else {
            break;
        }
        assert!(calls < 100, "paging did not terminate");
    }

    assert!(calls > 1, "120 entries should require multiple calls");
    assert_eq!(total_removed, 120);
    assert_eq!(client.get_history_count(&player_id), 0);
    assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(
        client.get_progress_root(&player_id),
        BytesN::from_array(&env, &[0u8; 32])
    );

    // One event per call; only the final call reports `more == false`.
    let events = purge_events(&env);
    assert_eq!(events.len(), calls);
    for i in 0..events.len() {
        let (_, more) = events.get(i).unwrap();
        assert_eq!(more, i + 1 < events.len());
    }
    assert_eq!(events.get(events.len() - 1).unwrap().1, false);
}

#[test]
fn test_purge_player_data_requires_admin() {
    let env = Env::default();
    let id = env.register(ProgressContract, (Address::generate(&env),));
    let client = ProgressContractClient::new(&env, &id);
    let player_id = 5u64;

    // No auths mocked: the admin auth requirement must fail.
    let result = client.try_purge_player_data(&player_id, &None);
    assert!(result.is_err(), "non-admin purge must fail auth");
}
