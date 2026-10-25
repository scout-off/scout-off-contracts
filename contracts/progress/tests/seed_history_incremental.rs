//! Issue #1618: `admin_seed_history` maintains the Merkle root incrementally
//! (O(log n) hashes per call) and writes the paged history layout.

use scoutchain_progress::{ProgressContract, ProgressContractClient, ProgressEntry};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{testutils::Address as _, xdr::ToXdr, Address, Bytes, BytesN, Env, Vec};

/// Budget for seeding the 500th history entry (see `ci/cpu-cost-budget.md`).
const SEED_500TH_ENTRY_CPU_BUDGET: u64 = 15_000_000;

fn setup() -> (Env, ProgressContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(ProgressContract, ());
    let client = ProgressContractClient::new(&env, &id);
    client.initialize(&Address::generate(&env));
    client.open_migration_window();
    (env, client)
}

fn make_entry(player_id: u64, index: u32, by: &Address) -> ProgressEntry {
    ProgressEntry {
        player_id,
        old_level: ProgressLevel::Unverified,
        new_level: ProgressLevel::VerifiedIdentity,
        updated_by: by.clone(),
        updated_at: 1_700_000_000 + index as u64 * 100,
        milestone_ref: index,
        ledger_sequence: 1000 + index,
    }
}

/// Independent RFC 6962 leaf hash: `H(0x00 || canonical entry bytes)`.
fn leaf(env: &Env, e: &ProgressEntry) -> BytesN<32> {
    let mut b = Bytes::new(env);
    b.push_back(0u8);
    b.extend_from_slice(&e.player_id.to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes()); // Unverified
    b.extend_from_slice(&1u32.to_be_bytes()); // VerifiedIdentity
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
