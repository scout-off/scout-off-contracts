//! Issue #1889: direct unit tests for `purge_player_data` (paged GDPR erasure).

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressEntry};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    testutils::{Address as _, Events},
    xdr::{ContractEventBody, ScVal},
    Address, BytesN, Env,
};

const PAGE: u32 = 50;

fn setup() -> (Env, ProgressContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(ProgressContract, (Address::generate(&env),));
    let client = ProgressContractClient::new(&env, &id);
    client.open_migration_window();
    (env, client)
}

/// Alternating advance/reset so the seeded history forms a valid level chain.
fn make_entry(player_id: u64, index: u32, by: &Address) -> ProgressEntry {
    let advance = index % 2 == 1;
    let (old_level, new_level) = if advance {
        (ProgressLevel::Unverified, ProgressLevel::VerifiedIdentity)
    } else {
        (ProgressLevel::VerifiedIdentity, ProgressLevel::Unverified)
    };
    ProgressEntry {
        player_id,
        old_level,
        new_level,
        updated_by: by.clone(),
        updated_at: 1_700_000_000 + index as u64 * 100,
        milestone_ref: if advance { index } else { 0 },
        ledger_sequence: 1000 + index,
    }
}

fn seed(env: &Env, client: &ProgressContractClient, player_id: u64, n: u32) {
    let by = Address::generate(env);
    for i in 1..=n {
        client.admin_seed_history(&player_id, &i, &make_entry(player_id, i, &by), &None);
    }
}

/// Returns `(entries_removed, more)` of the single `player_data_purged` event
/// emitted by the latest invocation (`events()` only covers the last call).
fn purge_event(env: &Env) -> (u32, bool) {
    let all = env.events().all();
    let found: Vec<_> = all
        .events()
        .iter()
        .filter_map(|e| match &e.body {
            ContractEventBody::V0(v0) => match v0.topics.first() {
                Some(ScVal::Symbol(s)) if AsRef::<[u8]>::as_ref(&s.0) == b"player_data_purged" => {
                    Some(v0.data.clone())
                }
                _ => None,
            },
        })
        .collect();
    assert_eq!(found.len(), 1, "exactly one purge event per call");
    let ScVal::Map(Some(map)) = &found[0] else {
        panic!("event data must be a map");
    };
    let field = |name: &str| {
        map.iter()
            .find(|kv| matches!(&kv.key, ScVal::Symbol(s) if AsRef::<[u8]>::as_ref(&s.0) == name.as_bytes()))
            .unwrap_or_else(|| panic!("missing field {name}"))
            .val
            .clone()
    };
    let removed = match field("entries_removed") {
        ScVal::U32(v) => v,
        other => panic!("unexpected entries_removed: {other:?}"),
    };
    let more = match field("more") {
        ScVal::Bool(b) => b,
        other => panic!("unexpected more: {other:?}"),
    };
    (removed, more)
}

fn assert_erased(client: &ProgressContractClient, player_id: u64) {
    assert_eq!(client.get_level(&player_id), ProgressLevel::Unverified);
    assert_eq!(client.get_history_count(&player_id), 0);
    assert_eq!(
        client.get_progress_root(&player_id),
        BytesN::from_array(&client.env, &[0u8; 32])
    );
}

#[test]
fn test_purge_player_without_history() {
    let (env, client) = setup();
    assert_eq!(client.purge_player_data(&1u64, &0u32), (0, false));
    assert_eq!(purge_event(&env), (0, false));
    assert_erased(&client, 1);
}

#[test]
fn test_purge_three_entries_single_call() {
    let (env, client) = setup();
    let player_id = 2u64;
    seed(&env, &client, player_id, 3);
    assert_eq!(client.get_history_count(&player_id), 3);
    assert_ne!(
        client.get_progress_root(&player_id),
        BytesN::from_array(&env, &[0u8; 32])
    );

    let (cursor, more) = client.purge_player_data(&player_id, &0u32);
    assert_eq!((cursor, more), (3, false));
    assert_erased(&client, player_id);
}

#[test]
fn test_purge_120_entries_is_paged_with_monotonic_cursor() {
    let (env, client) = setup();
    let player_id = 3u64;
    let total = 120u32;
    seed(&env, &client, player_id, total);
    assert_eq!(client.get_history_count(&player_id), total);
    // A full page deletes ~PAGE * 2 + 3 ledger entries, above the simulated
    // per-transaction write-entry limit; this test targets paging semantics.
    let _ = env.host().set_invocation_resource_limits(None);
    env.cost_estimate().budget().reset_unlimited();

    let mut cursor = 0u32;
    let mut calls = 0u32;
    loop {
        let (next, more) = client.purge_player_data(&player_id, &cursor);
        calls += 1;
        assert!(next > cursor, "cursor must be strictly monotonic");
        assert_eq!(next, (cursor + PAGE).min(total));
        let (_, ev_more) = purge_event(&env);
        assert_eq!(ev_more, more, "event `more` must match return value");
        cursor = next;
        if more {
            // Counter is removed only on the final page.
            assert_eq!(client.get_history_count(&player_id), total);
        } else {
            break;
        }
    }
    assert_eq!(calls, 3);
    assert_eq!(cursor, total);
    assert_erased(&client, player_id);
}

#[test]
fn test_purge_event_per_call_reports_removed_count() {
    let (env, client) = setup();
    let player_id = 4u64;
    seed(&env, &client, player_id, 3);

    client.purge_player_data(&player_id, &0u32);
    let (removed, more) = purge_event(&env);
    assert!(!more);
    // PlayerLevel + HistoryRoot + 3 entries + HistoryPage(0) + HistoryCounter.
    assert!(removed >= 5, "unexpectedly few removals: {removed}");

    // Idempotent re-run: nothing left to remove.
    assert_eq!(client.purge_player_data(&player_id, &0u32), (0, false));
    assert_eq!(purge_event(&env), (0, false));
}

#[test]
fn test_purge_requires_admin_auth() {
    // No `mock_all_auths`: the admin has not authorised the call.
    let env = Env::default();
    let id = env.register(ProgressContract, (Address::generate(&env),));
    let client = ProgressContractClient::new(&env, &id);
    assert!(client.try_purge_player_data(&1u64, &0u32).is_err());
}
