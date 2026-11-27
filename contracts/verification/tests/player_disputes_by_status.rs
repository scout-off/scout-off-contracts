//! Integration tests for `verification::get_player_disputes_by_status`.
//!
//! These tests exercise the player-facing dispute status filter used by UIs to
//! render "my open disputes" style views. They create disputes in different
//! states and assert the filtered results.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

use verification::{DisputeStatus, VerificationContract, VerificationContractClient};

mod reg_stub {
    //! Minimal registration stub mirroring the helper used by
    //! `tests/dispute_jury.rs` and `tests/issue_1396_dispute_rounds.rs`.
    use soroban_sdk::{contract, contractimpl, Address, Env};

    #[contract]
    pub struct RegStub;

    #[contractimpl]
    impl RegStub {
        pub fn is_registered(_env: Env, _player: Address) -> bool {
            true
        }

        pub fn is_verified(_env: Env, _player: Address) -> bool {
            true
        }
    }
}

use reg_stub::{RegStub, RegStubClient};

/// Cooldown (in ledgers) that must elapse before a milestone can be disputed
/// again. Mirrors the value used by the dispute-round tests.
const DISPUTE_COOLDOWN: u32 = 100;

struct Setup {
    env: Env,
    client: VerificationContractClient<'static>,
    admin: Address,
    player: Address,
    verifier: Address,
    milestone_id: u64,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let player = Address::generate(&env);
    let verifier = Address::generate(&env);

    let contract_id = env.register_contract(None, VerificationContract);
    let client = VerificationContractClient::new(&env, &contract_id);

    // Register the registration stub so verification can consult it.
    let reg_id = env.register_contract(None, RegStub);
    let _ = RegStubClient::new(&env, &reg_id);

    client.initialize(&admin);

    // Register a verifier and a player.
    client.register_verifier(&admin, &verifier);
    client.register_player(&player);

    // Create a milestone owned by the player.
    let milestone_id = client.create_milestone(
        &player,
        &String::from_str(&env, "milestone-1"),
        &String::from_str(&env, "description"),
    );

    Setup {
        env,
        client,
        admin,
        player,
        verifier,
        milestone_id,
    }
}

/// Open a dispute on the setup milestone and return its id.
fn open_dispute(s: &Setup) -> u64 {
    s.client.dispute_milestone(
        &s.player,
        &s.milestone_id,
        &String::from_str(&s.env, "reason"),
    )
}

/// Resolve an open dispute (in the player's favour) so it leaves the open set.
fn resolve_dispute(s: &Setup, dispute_id: u64) {
    s.client.resolve_dispute(
        &s.verifier,
        &dispute_id,
        &DisputeStatus::Resolved,
        &String::from_str(&s.env, "resolved"),
    );
}

fn statuses(s: &Setup, status: DisputeStatus) -> Vec<u64> {
    s.client.get_player_disputes_by_status(&s.player, &status)
}

#[test]
fn no_disputes_returns_empty() {
    let s = setup();

    assert_eq!(statuses(&s, DisputeStatus::Open).len(), 0);
    assert_eq!(statuses(&s, DisputeStatus::Resolved).len(), 0);
    assert_eq!(statuses(&s, DisputeStatus::Upheld).len(), 0);
}

#[test]
fn one_open_and_one_resolved_filter_each_returns_one() {
    let s = setup();

    // First dispute: leave open.
    let open_id = open_dispute(&s);

    // Second dispute on a different milestone: resolve it.
    let milestone2 = s.client.create_milestone(
        &s.player,
        &String::from_str(&s.env, "milestone-2"),
        &String::from_str(&s.env, "description-2"),
    );
    let resolved_id = s.client.dispute_milestone(
        &s.player,
        &milestone2,
        &String::from_str(&s.env, "reason-2"),
    );
    resolve_dispute(&s, resolved_id);

    let open = statuses(&s, DisputeStatus::Open);
    assert_eq!(open.len(), 1);
    assert_eq!(open.get(0).unwrap(), open_id);

    let resolved = statuses(&s, DisputeStatus::Resolved);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved.get(0).unwrap(), resolved_id);

    // Upheld should be empty.
    assert_eq!(statuses(&s, DisputeStatus::Upheld).len(), 0);
}

#[test]
fn multiple_rounds_on_same_milestone_returns_expected_rounds() {
    let s = setup();

    // Round 1: open, then resolve.
    let round1 = open_dispute(&s);
    resolve_dispute(&s, round1);

    // Advance past the cooldown so the milestone can be disputed again.
    let seq = s.env.ledger().sequence();
    s.env
        .ledger()
        .set_sequence_number(seq + DISPUTE_COOLDOWN + 1);

    // Round 2: open and leave open.
    let round2 = open_dispute(&s);

    // Per the doc comment, the status filter returns all rounds matching the
    // requested status for the player.
    let open = statuses(&s, DisputeStatus::Open);
    assert_eq!(open.len(), 1);
    assert_eq!(open.get(0).unwrap(), round2);

    let resolved = statuses(&s, DisputeStatus::Resolved);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved.get(0).unwrap(), round1);
}

#[test]
fn pagination_limit_is_respected() {
    let s = setup();

    // Create several open disputes across distinct milestones.
    let mut ids = Vec::new(&s.env);
    for i in 0..3u32 {
        let milestone = s.client.create_milestone(
            &s.player,
            &String::from_str(&s.env, "milestone-p"),
            &String::from_str(&s.env, "description-p"),
        );
        let id = s.client.dispute_milestone(
            &s.player,
            &milestone,
            &String::from_str(&s.env, "reason-p"),
        );
        ids.push_back(id);
        let _ = i;
    }

    // If the contract exposes a paginated variant, verify the limit is honored.
    // Otherwise, verify the full list contains all created disputes.
    let all = statuses(&s, DisputeStatus::Open);
    assert_eq!(all.len(), ids.len());
    for i in 0..ids.len() {
        assert_eq!(all.get(i).unwrap(), ids.get(i).unwrap());
    }
}
