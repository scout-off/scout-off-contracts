//! Ring-buffer global milestone index tests (issue #704).
//!
//! Three concrete acceptance criteria:
//!
//! 1. **Budget flatness** — `approve_milestone`'s global-index maintenance
//!    code path is O(1) regardless of fill level.  Proved via
//!    `env.cost_estimate()` by sampling CPU-instruction cost at write ~10,
//!    ~500, and ~1000.  The measured calls use fresh validators (zero prior
//!    milestones) so the growing `ValidatorMilestones` vector does not
//!    pollute the global-index-specific cost signal.
//!
//! 2. **Wraparound pagination** — approving more than MAX_GLOBAL_MILESTONE_INDEX
//!    milestones and then requesting a page that spans the ring-buffer wrap
//!    boundary returns entries in the correct insertion order with no
//!    duplicates, gaps, or off-by-one errors at the wrap point.
//!
//! 3. **Eviction** — once the ring buffer is full, the oldest surviving entry
//!    is correctly evicted on the next write and is no longer retrievable via
//!    any page of `get_global_milestone_index`.
//!
//! ## Validator pool design
//!
//! The contract caps the validator registry at MAX_VALIDATORS=100.
//!
//! For **fill** calls (unmeasured warm-up):
//!   We pre-register a pool of POOL_SIZE validators and cycle through them
//!   using `seed % POOL_SIZE` for validator selection with `player_id = seed+1`
//!   (unique per seed).  Since each (validator, player_id) pair appears at most
//!   once, neither the 100-validator cap nor the 5-milestones/player/validator
//!   cap is hit across up to POOL_SIZE * ∞ unique players.
//!
//! For **measured** calls (budget sampling):
//!   A fresh, zero-history validator is registered for each measured call so
//!   the `ValidatorMilestones` append cost is always O(1) for a brand-new
//!   entry — isolating the global-index write cost from the (legitimate but
//!   separate) O(n) growth of per-validator milestone history.

use scoutchain_verification::{GlobalMilestoneEntry, VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

/// Pool size for fill validators.  Must be < MAX_VALIDATORS (100) to leave
/// room for the measurement validators.  Using 90 leaves 10 free slots for
/// measurement calls (3 per budget-flatness test).
const POOL_SIZE: u64 = 90;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn setup() -> (Env, VerificationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

/// Register a single new validator and return its address.
/// Panics (via contract error) if the 100-validator cap is exceeded — callers
/// must stay within their quota.
fn register_one(env: &Env, client: &VerificationContractClient<'_>) -> Address {
    let v = Address::generate(env);
    client.register_validator(
        &v,
        &String::from_str(env, "UEFA-A-License-2026"),
        &String::from_str(env, "Test Academy"),
        &Vec::new(env),
    );
    v
}

/// Pre-register `POOL_SIZE` validators and return them.
fn build_pool(env: &Env, client: &VerificationContractClient<'_>) -> std::vec::Vec<Address> {
    let mut pool = std::vec::Vec::with_capacity(POOL_SIZE as usize);
    for _ in 0..POOL_SIZE {
        pool.push(register_one(env, client));
    }
    pool
}

/// Build a distinct CIDv0 evidence hash for a numeric seed.
///
/// Base is exactly 44 valid base58btc characters; appending two seed-derived
/// characters gives exactly 46, which is the required CIDv0 length.
fn cid_for(env: &Env, seed: u64) -> String {
    const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // 44-char base — CIDv0 requires exactly 46 chars (44 + 2 seed suffix).
    let base = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygp";
    let c1 = B58[((seed / 58) % 58) as usize] as char;
    let c2 = B58[(seed % 58) as usize] as char;
    let s = format!("{base}{c1}{c2}");
    debug_assert_eq!(s.len(), 46, "CIDv0 must be exactly 46 chars");
    String::from_str(env, &s)
}

/// Approve one milestone using the pre-built validator pool.
///
/// Validator: `pool[seed % POOL_SIZE]`  Player: `seed + 1` (unique per seed).
/// Since (validator, player_id) is unique for each seed, neither the
/// validator-count cap nor the per-player-per-validator milestone cap is hit.
fn fill(
    env: &Env,
    client: &VerificationContractClient<'_>,
    pool: &[Address],
    seed: u64,
) {
    let validator = &pool[(seed % POOL_SIZE) as usize];
    let player_id = seed + 1;
    client.approve_milestone(
        validator,
        &player_id,
        &String::from_str(env, "ring buffer fill milestone"),
        &cid_for(env, seed),
        &None,
    );
}

/// Collect every (player_id, milestone_index) the global index currently
/// holds by draining pages until empty.
fn drain_index(
    _env: &Env,
    client: &VerificationContractClient<'_>,
) -> std::vec::Vec<(u64, u32)> {
    let mut out = std::vec::Vec::new();
    let mut offset: u32 = 0;
    loop {
        let page = client.get_global_milestone_index(&offset, &50u32);
        let n = page.entries.len();
        for i in 0..n {
            let e = page.entries.get(i).unwrap();
            out.push((e.player_id, e.milestone_index));
        }
        offset += n;
        if n < 50 || offset >= page.total {
            break;
        }
    }
    out
}

// ─── test 1: budget flatness ──────────────────────────────────────────────────

/// Prove the global-index write path is O(1) in total ring-buffer size.
///
/// We measure `approve_milestone`'s CPU cost at three ring-buffer fill levels
/// using **independent contract instances** so that background ledger-state
/// growth (EvidenceUsed entries, ValidatorPlayers vectors, etc.) is held
/// constant across all three samples.  Each instance:
///   1. Pre-registers one validator pool (POOL_SIZE validators, no milestone
///      history beyond the fill calls).
///   2. Fills the ring buffer to the desired level using pooled validators and
///      unique player_ids.
///   3. Registers one fresh measurement validator (zero milestone history).
///   4. Resets the budget and calls `approve_milestone` once with the fresh
///      validator to measure only the incremental cost of that call at that
///      fill level.
///
/// Because the only difference between the three instances is the value of
/// `GlobalMilestoneWriteHead` (and the corresponding persistent slots), any
/// O(n) growth in the global-index write path shows up as cost proportional to
/// the fill level.  A 20× tolerance absorbs legitimate Soroban-simulator
/// variance while clearly catching any true O(n) regression.
///
/// Fill levels tested:
///   • 10 entries   (well below capacity)
///   • 500 entries  (at capacity — ring buffer full, evictions active)
///   • 1 000 entries (ring wrapped twice)
///
/// Seed ranges per instance (no overlap between fill and measure):
///   • Fill seeds 0..fill_count  → player_ids 1..fill_count+1
///   • Measure seed fill_count + 1_000_000 → safely outside fill range
#[test]
fn cost_global_milestone_index_write_is_flat() {
    /// Measure CPU cost of one `approve_milestone` call after pre-filling
    /// the ring buffer to `fill_count` entries in a fresh contract instance.
    fn measure_at_fill(fill_count: u64) -> u64 {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(VerificationContract, ());
        let client = VerificationContractClient::new(&env, &id);
        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Build the validator pool and fill the ring buffer.
        let pool = build_pool(&env, &client);
        for seed in 0..fill_count {
            fill(&env, &client, &pool, seed);
        }

        // Register a fresh measurement validator (zero prior history).
        let fresh_validator = register_one(&env, &client);
        let measure_seed = fill_count + 1_000_000;
        let player_id = measure_seed + 1;

        // Reset budget immediately before the measured call.
        env.cost_estimate().budget().reset_default();
        client.approve_milestone(
            &fresh_validator,
            &player_id,
            &String::from_str(&env, "budget measurement"),
            &cid_for(&env, measure_seed),
            &None,
        );
        env.cost_estimate().budget().cpu_instruction_cost()
    }

    let cost_early     = measure_at_fill(10);
    let cost_full      = measure_at_fill(500);
    let cost_post_wrap = measure_at_fill(1_000);

    println!("ring_buffer budget flatness: cost_early  (fill ~10)    = {cost_early} cpu instructions");
    println!("ring_buffer budget flatness: cost_full   (fill ~500)   = {cost_full} cpu instructions");
    println!("ring_buffer budget flatness: cost_wrap   (fill ~1000)  = {cost_post_wrap} cpu instructions");

    // The ring-buffer global-index write is O(1): one instance read
    // (write_head), one persistent write (slot), one instance write
    // (write_head+1).  A 20× tolerance still catches true O(n) regressions (old Vec-rewrite showed 100×+ growth)
    // while still catching any O(n) regression (which would show up as
    // ~100× growth at fill 1000 vs fill 10).
    let budget = cost_early.saturating_mul(20);
    assert!(
        cost_full <= budget,
        "ring_buffer: cost at fill ~500 ({cost_full}) exceeds 20× early cost ({cost_early}). \
         The global-index write must be O(1) — an O(n) path may have crept back in."
    );
    assert!(
        cost_post_wrap <= budget,
        "ring_buffer: cost at fill ~1000 ({cost_post_wrap}) exceeds 20× early cost ({cost_early}). \
         The global-index write must be O(1) — an O(n) path may have crept back in."
    );
}

// ─── test 2: wraparound boundary pagination ───────────────────────────────────

/// Approve MAX_GLOBAL_MILESTONE_INDEX + 20 milestones (520 total) and request
/// a page that straddles the physical ring-buffer wraparound boundary.
///
/// After 520 writes (capacity = 500):
///   write_head = 520
///   oldest_slot = 520 % 500 = 20
///   surviving entries = writes #20..#519 → player_ids 21..520
///
/// Page request: offset=470, limit=50 → returns the last 30 surviving entries.
/// The underlying slot read crosses the slot=499 → slot=0 boundary.
///
/// Assertions:
///   - Page contains exactly 30 entries.
///   - All player_ids are distinct.
///   - Entries are in strict ascending insertion order.
///   - First and last player_ids match the expected surviving range.
///   - Full drain (all pages) returns exactly 500 distinct ordered entries.
#[test]
fn test_wraparound_boundary_pagination() {
    const CAP: u64 = 500;
    const EXTRA: u64 = 20;
    const TOTAL: u64 = CAP + EXTRA;

    let (env, client) = setup();
    let pool = build_pool(&env, &client);

    // Approve TOTAL milestones using the validator pool.
    for seed in 0..TOTAL {
        fill(&env, &client, &pool, seed);
    }

    // total should be capped at CAP.
    let probe = client.get_global_milestone_index(&0u32, &1u32);
    assert_eq!(probe.total, CAP as u32, "total should be capped at {CAP}");

    // Fetch a page straddling the physical slot wrap (offset=470, limit=50).
    let page = client.get_global_milestone_index(&470u32, &50u32);
    assert_eq!(
        page.entries.len(),
        30,
        "only 30 entries remain after offset=470 in a live_count=500 buffer"
    );

    // All player_ids must be distinct.
    let mut seen: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for i in 0..page.entries.len() {
        let e: GlobalMilestoneEntry = page.entries.get(i).unwrap();
        assert!(
            seen.insert(e.player_id),
            "duplicate player_id {} at position {i} in wraparound page",
            e.player_id
        );
    }

    // Entries must be in strict ascending player_id order (= insertion order).
    for i in 1..page.entries.len() {
        let prev: GlobalMilestoneEntry = page.entries.get(i - 1).unwrap();
        let curr: GlobalMilestoneEntry = page.entries.get(i).unwrap();
        assert!(
            curr.player_id > prev.player_id,
            "entries out of order at position {i}: {} followed by {}",
            prev.player_id, curr.player_id
        );
    }

    // The 30 entries should be the last 30 of the 500 live entries.
    // Surviving entries: seeds 20..519 → player_ids 21..520.
    // offset=470 skips 470 → player_ids 21+470=491 .. 520.
    let expected_first = EXTRA + 1 + 470; // 491
    let expected_last  = EXTRA + 1 + 499; // 520
    let first_pid: GlobalMilestoneEntry = page.entries.get(0).unwrap();
    let last_pid:  GlobalMilestoneEntry = page.entries.get(page.entries.len() - 1).unwrap();
    assert_eq!(
        first_pid.player_id, expected_first,
        "first entry in wraparound page should be player_id {expected_first}"
    );
    assert_eq!(
        last_pid.player_id, expected_last,
        "last entry in wraparound page should be player_id {expected_last}"
    );

    // Full drain: 500 distinct ordered entries, oldest first.
    let all = drain_index(&env, &client);
    assert_eq!(all.len(), CAP as usize, "full drain should return exactly {CAP} entries");

    let mut all_pids: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for &(pid, _) in &all {
        assert!(all_pids.insert(pid), "duplicate player_id {pid} in full drain");
    }

    for i in 1..all.len() {
        assert!(
            all[i].0 > all[i - 1].0,
            "full drain not ordered at position {i}: {} followed by {}",
            all[i - 1].0, all[i].0
        );
    }

    // Oldest surviving = seed 20 → player_id 21.
    assert_eq!(all[0].0, EXTRA + 1, "oldest surviving entry should be player_id {}", EXTRA + 1);
    // Newest surviving = seed 519 → player_id 520.
    assert_eq!(all[CAP as usize - 1].0, TOTAL, "newest surviving entry should be player_id {TOTAL}");
}

// ─── test 3: oldest-entry eviction ───────────────────────────────────────────

/// Once the ring buffer is full (500 entries), the next write evicts the oldest
/// entry and it disappears from every page of `get_global_milestone_index`.
///
/// Steps:
///   1. Fill the buffer to exactly 500.  Oldest = player_id 1 (seed 0).
///   2. Approve write #501.  This overwrites slot 0 (oldest slot when full).
///   3. Assert oldest is now player_id 2.
///   4. Drain all pages and confirm player_id 1 is gone.
///   5. Confirm the new write (player_id 501) is the newest entry.
#[test]
fn test_oldest_entry_evicted_when_buffer_full() {
    const CAP: u32 = 500;
    let (env, client) = setup();
    let pool = build_pool(&env, &client);

    // Fill to capacity: seeds 0..499 → player_ids 1..500.
    for seed in 0u64..CAP as u64 {
        fill(&env, &client, &pool, seed);
    }

    // Snapshot: oldest entry = offset 0 → player_id 1.
    let before = client.get_global_milestone_index(&0u32, &1u32);
    assert_eq!(before.total, CAP);
    let oldest_before: GlobalMilestoneEntry = before.entries.get(0).unwrap();
    assert_eq!(oldest_before.player_id, 1, "oldest before eviction should be player_id 1");

    // 501st write: seed=500 → player_id 501.
    let eviction_seed: u64 = CAP as u64;
    fill(&env, &client, &pool, eviction_seed);
    let new_player_id: u64 = eviction_seed + 1; // = 501

    // total stays at CAP.
    let after = client.get_global_milestone_index(&0u32, &1u32);
    assert_eq!(after.total, CAP, "total should remain {CAP} after eviction");

    // Oldest is now player_id 2.
    let oldest_after: GlobalMilestoneEntry = after.entries.get(0).unwrap();
    assert_eq!(
        oldest_after.player_id, 2,
        "after eviction, oldest should be player_id 2"
    );

    // Full drain: player_id 1 must not appear.
    let all = drain_index(&env, &client);
    assert_eq!(all.len(), CAP as usize, "full drain should return {CAP} entries after eviction");

    assert!(
        !all.iter().any(|&(pid, _)| pid == 1),
        "evicted entry (player_id=1) must not be retrievable via any page"
    );

    // The newest entry should be the 501st write.
    assert_eq!(
        all.last().unwrap().0, new_player_id,
        "newest entry should be player_id {new_player_id}"
    );
}
