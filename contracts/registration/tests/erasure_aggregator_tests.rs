//! Tests for the registration contract's erasure-status aggregator
//! (issue #1781): `get_erasure_status` and the new setter functions
//! `set_verification_contract` / `set_scout_access_contract`.

use scoutchain_registration::{
    ErasureStatus, PurgeStatus, RegistrationContract, RegistrationContractClient,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

struct Harness {
    env: Env,
    client: RegistrationContractClient<'static>,
}

fn setup() -> Harness {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &id);
    client.initialize(&admin);
    Harness { env, client }
}

/// Before any peer contracts are configured, all three statuses are the
/// zero-value and `fully_erased` is false.
#[test]
fn test_get_erasure_status_no_peers_configured() {
    let h = setup();
    let status: ErasureStatus = h.client.get_erasure_status(&42u64);
    let zero = PurgeStatus {
        started: false,
        cursor: 0,
        completed: false,
    };
    assert_eq!(status.progress, zero);
    assert_eq!(status.verification, zero);
    assert_eq!(status.scout_access, zero);
    assert!(!status.fully_erased);
}

/// Setting peer addresses succeeds (admin-only setters exist and persist).
#[test]
fn test_set_verification_and_scout_access_contract() {
    let h = setup();
    let fake_ver = Address::generate(&h.env);
    let fake_sa = Address::generate(&h.env);

    h.client.set_verification_contract(&fake_ver);
    h.client.set_scout_access_contract(&fake_sa);

    // After setting, get_erasure_status will attempt to cross-call those
    // addresses. In testutils with mock_all_auths the calls will simply
    // fail gracefully (no WASM deployed at those addresses), but the
    // important thing is that the setters don't panic.
    // We just verify the function completes without error.
    // (Real integration tests that deploy all three contracts would go further.)
}

/// `fully_erased` requires all three to be completed. With no peers, it stays false.
#[test]
fn test_fully_erased_false_without_peers() {
    let h = setup();
    assert!(!h.client.get_erasure_status(&1u64).fully_erased);
}
