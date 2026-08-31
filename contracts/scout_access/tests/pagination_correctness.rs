//! Pagination correctness tests for `get_scout_contacts_page` (issue #619).
//!
//! Acceptance-criteria coverage:
//!
//! 1. Empty pages past the end of the list return `entries = []` and the
//!    correct `total`.
//! 2. Exact boundary: a page starting at `offset == total` is empty.
//! 3. A page where `offset + limit == total` returns the last entry and no
//!    more.
//! 4. `limit` is silently clamped to 50 — requesting 200 entries never
//!    returns more than 50.
//! 5. Walking all pages with `limit = 10` covers every contacted player_id
//!    exactly once and reports the same `total` on every page.
//!
//! These tests use `get_scout_contacts_page` as the canonical representative
//! for the shared pagination pattern.  Equivalent logic is exercised for
//! `get_validator_milestones_page_v2` and `get_validator_players_page` in
//! `contracts/verification/tests/pagination_correctness.rs`.

use scoutchain_scout_access::{FeeConfig, ScoutAccessContract, ScoutAccessContractClient, SubscriptionTier};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn default_fees() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 50,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
    }
}

struct Harness {
    env: Env,
    xlm: Address,
    client: ScoutAccessContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let admin = Address::generate(&env);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &id);
    client.initialize(&admin, &xlm, &default_fees());

    Harness { env, xlm, client }
}

/// Subscribe a scout to Elite and fund them enough for `n` contacts.
fn subscribe_elite(h: &Harness, scout: &Address, contacts_needed: u32) {
    // Elite sub: 7_000_000, contact fee: 100_000 each
    let needed: i128 = 7_000_000 + 100_000 * contacts_needed as i128 + 1_000_000;
    StellarAssetClient::new(&h.env, &h.xlm).mint(scout, &needed);
    h.client.subscribe(scout, &SubscriptionTier::Elite);
}

/// Contact `count` players (player IDs 1..=count) via `pay_to_contact`.
fn contact_players(h: &Harness, scout: &Address, count: u32) {
    for i in 1..=count {
        h.client.pay_to_contact(scout, &(i as u64));
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Offset past the end returns an empty entries vec and the correct total.
#[test]
fn test_empty_page_past_end() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe_elite(&h, &scout, 5);
    contact_players(&h, &scout, 5); // player IDs 1–5

    // offset = 5 == total → empty page
    let page = h.client.get_scout_contacts_page(&scout, &5u32, &10u32);
    assert_eq!(page.total, 5, "total must equal number of contacts");
    assert_eq!(page.entries.len(), 0, "entries past the end must be empty");

    // offset > total
    let page_far = h.client.get_scout_contacts_page(&scout, &100u32, &10u32);
    assert_eq!(page_far.total, 5);
    assert_eq!(page_far.entries.len(), 0);
}

/// Exact boundary: offset + limit == total returns the last chunk and nothing more.
#[test]
fn test_exact_boundary_offset_plus_limit_eq_total() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe_elite(&h, &scout, 6);
    contact_players(&h, &scout, 6); // player IDs 1–6, total = 6

    // offset=4, limit=2 → entries at indices 4 and 5 (player IDs 5 and 6)
    let page = h.client.get_scout_contacts_page(&scout, &4u32, &2u32);
    assert_eq!(page.total, 6);
    assert_eq!(page.entries.len(), 2, "should return exactly 2 entries at boundary");
    assert_eq!(page.entries.get(0).unwrap(), 5u64);
    assert_eq!(page.entries.get(1).unwrap(), 6u64);
}

/// Limit is clamped at 50: requesting 200 never returns more than 50.
#[test]
fn test_limit_clamped_at_50() {
    let h = setup();
    let scout = Address::generate(&h.env);
    // Contact 55 players (needs a higher pro_contact_limit; we set 50 in
    // default_fees but Elite has no contact cap so all 55 succeed).
    subscribe_elite(&h, &scout, 55);
    contact_players(&h, &scout, 55);

    // Requesting limit=200 should return at most 50.
    let page = h.client.get_scout_contacts_page(&scout, &0u32, &200u32);
    assert_eq!(page.total, 55, "total must reflect all 55 contacts");
    assert!(
        page.entries.len() <= 50,
        "entries must never exceed the 50-entry cap (got {})",
        page.entries.len()
    );
    assert_eq!(page.entries.len(), 50, "first 50 entries returned for offset=0");
}

/// Walk all pages with limit=10 and confirm every player ID is seen exactly
/// once, in insertion order, and `total` is consistent across pages.
#[test]
fn test_walk_all_pages_exact_coverage() {
    let h = setup();
    let scout = Address::generate(&h.env);
    const TOTAL: u32 = 25;
    subscribe_elite(&h, &scout, TOTAL);
    contact_players(&h, &scout, TOTAL);

    let mut seen: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&h.env);
    let mut offset = 0u32;
    let limit = 10u32;

    loop {
        let page = h.client.get_scout_contacts_page(&scout, &offset, &limit);
        assert_eq!(page.total, TOTAL, "total must be consistent across pages");
        if page.entries.is_empty() {
            break;
        }
        for i in 0..page.entries.len() {
            seen.push_back(page.entries.get(i).unwrap());
        }
        offset += limit;
    }

    assert_eq!(
        seen.len(),
        TOTAL,
        "all {TOTAL} contacts must be seen exactly once"
    );
    // Verify insertion order: player IDs 1 through TOTAL in sequence.
    for i in 0..seen.len() {
        assert_eq!(
            seen.get(i).unwrap(),
            (i + 1) as u64,
            "player IDs must appear in contact order (oldest first)"
        );
    }
}

/// A scout with no contacts returns total=0 and empty entries.
#[test]
fn test_no_contacts_returns_empty() {
    let h = setup();
    let scout = Address::generate(&h.env);

    let page = h.client.get_scout_contacts_page(&scout, &0u32, &10u32);
    assert_eq!(page.total, 0);
    assert_eq!(page.entries.len(), 0);
}

/// First page (offset=0) returns entries in contact order (oldest first).
#[test]
fn test_first_page_insertion_order() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe_elite(&h, &scout, 3);
    // Contact players in order 10, 20, 30
    h.client.pay_to_contact(&scout, &10u64);
    h.client.pay_to_contact(&scout, &20u64);
    h.client.pay_to_contact(&scout, &30u64);

    let page = h.client.get_scout_contacts_page(&scout, &0u32, &10u32);
    assert_eq!(page.total, 3);
    assert_eq!(page.entries.len(), 3);
    assert_eq!(page.entries.get(0).unwrap(), 10u64, "oldest contact first");
    assert_eq!(page.entries.get(1).unwrap(), 20u64);
    assert_eq!(page.entries.get(2).unwrap(), 30u64);
}

/// limit=0 returns an empty page but the correct total.
#[test]
fn test_zero_limit_returns_empty_with_total() {
    let h = setup();
    let scout = Address::generate(&h.env);
    subscribe_elite(&h, &scout, 3);
    contact_players(&h, &scout, 3);

    let page = h.client.get_scout_contacts_page(&scout, &0u32, &0u32);
    assert_eq!(page.total, 3, "total still reflects 3 contacts");
    assert_eq!(page.entries.len(), 0, "limit=0 returns zero entries");
}
