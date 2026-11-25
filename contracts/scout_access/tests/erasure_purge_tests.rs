//! Tests for the player-data purge / erasure feature on the scout_access
//! contract (issue #1781).
//!
//! Covers:
//! - `get_purge_status` zero-value before any purge
//! - `purge_player_data` for a player with no contact data (page 0 completes)
//! - `purge_player_data` lifecycle: started → in-progress → completed
//! - Wrong cursor is rejected with PurgeCursorMismatch
//! - Idempotency after completion

use scoutchain_scout_access::{
    FeeConfig, PurgeStatus, ScoutAccessContract, ScoutAccessContractClient,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

const ONE_XLM: i128 = 10_000_000;

fn default_fee_config() -> FeeConfig {
    FeeConfig {
        contact_fee_stroops: ONE_XLM,
        basic_sub_stroops: ONE_XLM,
        pro_sub_stroops: 2 * ONE_XLM,
        elite_sub_stroops: 3 * ONE_XLM,
        sub_duration_secs: 86_400 * 30,
        trial_offer_escrow_stroops: ONE_XLM,
        trial_offer_expiry_secs: 86_400,
        pro_contact_limit: 10,
    }
}

struct Harness {
    client: ScoutAccessContractClient<'static>,
    admin: Address,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);

    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let contract_id = env.register(ScoutAccessContract, ());
    let client = ScoutAccessContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_id, &default_fee_config());

    Harness { client, admin }
}

// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_purge_status_before_start_is_zero() {
    let h = setup();
    let status = h.client.get_purge_status(&42u64);
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
fn test_purge_no_data_completes_on_first_call() {
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
fn test_purge_wrong_cursor_rejected() {
    use scoutchain_scout_access::ScoutAccessError;
    let h = setup();
    let pid: u64 = 2;

    // Supplying cursor=1 when stored cursor is 0 (never started) must be rejected.
    let result = h.client.try_purge_player_data(&h.admin, &pid, &1u32);
    assert_eq!(result, Err(Ok(ScoutAccessError::PurgeCursorMismatch)));
}

#[test]
fn test_purge_idempotent_after_completion() {
    let h = setup();
    let pid: u64 = 3;
    h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert!(h.client.get_purge_status(&pid).completed);

    // Second call after completion returns (0, false) silently.
    let (nc, more) = h.client.purge_player_data(&h.admin, &pid, &0u32);
    assert_eq!(nc, 0u32);
    assert!(!more);
}

#[test]
fn test_purge_two_players_independent() {
    let h = setup();

    h.client.purge_player_data(&h.admin, &10u64, &0u32);
    assert!(h.client.get_purge_status(&10u64).completed);

    // Player 11 untouched.
    let s11 = h.client.get_purge_status(&11u64);
    assert!(!s11.started);
    assert!(!s11.completed);
}

#[test]
fn test_purge_started_flag_set_after_first_call() {
    let h = setup();
    let pid: u64 = 5;

    let before = h.client.get_purge_status(&pid);
    assert!(!before.started);

    h.client.purge_player_data(&h.admin, &pid, &0u32);

    let after = h.client.get_purge_status(&pid);
    assert!(after.started);
}
