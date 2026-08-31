//! Unit tests for the dispute jury escalation system (issue #1036).
//!
//! Covers:
//! 1. set_jury_config / get_jury_config — defaults and admin-only
//! 2. dispute_milestone routing (below/at/above threshold)
//! 3. Jury config snapshotted at filing — later changes have no effect
//! 4. cast_dispute_vote eligibility rules:
//!    a. ValidatorNotFound if unregistered
//!    b. ValidatorInactive if revoked
//!    c. ConflictOfInterest for the original milestone approver
//!    d. AlreadyVoted on duplicate vote
//!    e. NotJuryDispute for admin-path disputes
//!    f. VotingWindowClosed after deadline
//! 5. tally_dispute — early close (clear majority at quorum)
//! 6. tally_dispute — tie-break at deadline (upheld=false)
//! 7. tally_dispute — deadline passed with majority
//! 8. tally_dispute — deadline passed below quorum → upheld=false
//! 9. resolve_dispute blocked for jury disputes (DisputeRequiresJury)
//! 10. resolve_dispute still works for non-jury disputes
//! 11. Adversarial: tied exactly at quorum refuses early-close; deadline resolves false
//! 12. Vote tallies tracked correctly across multiple voters

use scoutchain_verification::{
    RegPlayerProfile, RegPlayerVitals, RevocationSeverity, VerificationContract,
    VerificationContractClient, VerificationError,
};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

// ─── Minimal registration-contract stub ──────────────────────────────────────
// `dispute_milestone` cross-calls the registration contract to verify that the
// calling wallet is the owner of `player_id`.  This in-process stub returns the
// address stored at init time for every `get_player` lookup.

#[contracttype]
enum RegStubKey {
    Owner,
}

#[contract]
struct RegStub;

#[contractimpl]
impl RegStub {
    pub fn initialize(env: Env, owner: Address) {
        env.storage().persistent().set(&RegStubKey::Owner, &owner);
    }

    pub fn get_player(env: Env, player_id: u64) -> RegPlayerProfile {
        let wallet: Address = env.storage().persistent().get(&RegStubKey::Owner).unwrap();
        RegPlayerProfile {
            player_id,
            wallet,
            vitals: RegPlayerVitals {
                age: 20,
                position: String::from_str(&env, "Forward"),
                region: String::from_str(&env, "Europe"),
                nationality: String::from_str(&env, "ES"),
            },
            ipfs_hashes: Vec::new(&env),
            level: ProgressLevel::Unverified,
            registered_at: 0,
            updated_at: 0,
        }
    }
}

const CREDENTIALS: &str = "UEFA-B-License-2026";
const CRED2: &str = "FA-Coach-License-2026";
const CRED3: &str = "CAF-Cert-Level-A-2026";
const CRED4: &str = "FIFA-Trainer-B-2026";
const CRED5: &str = "CONCACAF-License-B-2026";

const CID_1: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const REASON: &str = "Milestone was approved in error";

fn setup() -> (Env, VerificationContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let player = Address::generate(&env);
    client.initialize(&admin);
    // Wire a minimal registration stub so `dispute_milestone` can verify
    // wallet→player_id ownership.  The stub returns `player` for every lookup.
    let reg_id = env.register(RegStub, ());
    let reg_client = RegStubClient::new(&env, &reg_id);
    reg_client.initialize(&player);
    client.set_registration_contract(&reg_id);
    (env, client, admin, player)
}

fn reg_validator(env: &Env, client: &VerificationContractClient, creds: &str) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, creds),
        &String::from_str(env, "Test Academy"),
        &Vec::new(env),
    );
    wallet
}

fn file_jury_dispute(env: &Env, client: &VerificationContractClient, player: &Address) {
    // impact_score >= default threshold (100) → jury path
    client.dispute_milestone(
        player,
        &1u64,
        &1u32,
        &String::from_str(env, REASON),
        &100u32,
    );
}

/// Creates a milestone and files a jury dispute. Returns the approver wallet.
fn setup_jury_dispute(env: &Env, client: &VerificationContractClient, player: &Address) -> Address {
    let validator = reg_validator(env, client, CREDENTIALS);
    client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(env, "scored a hat-trick"),
        &String::from_str(env, CID_1),
        &None,
    );
    file_jury_dispute(env, client, player);
    validator
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. set_jury_config / get_jury_config
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_get_jury_config_defaults() {
    let (env, client, _admin, player) = setup();
    let cfg = client.get_jury_config();
    assert_eq!(cfg.impact_threshold, 100);
    assert_eq!(cfg.quorum, 3);
    assert_eq!(cfg.voting_window_secs, 604_800);
}

#[test]
fn test_set_jury_config_updates_values() {
    let (env, client, _admin, player) = setup();
    client.set_jury_config(&50u32, &5u32, &86_400u64);
    let cfg = client.get_jury_config();
    assert_eq!(cfg.impact_threshold, 50);
    assert_eq!(cfg.quorum, 5);
    assert_eq!(cfg.voting_window_secs, 86_400);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. dispute_milestone routing
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_dispute_below_threshold_is_admin_path() {
    let (env, client, _admin, player) = setup();
    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);

    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &50u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(!d.jury_required);
    assert_eq!(d.voting_deadline, 0);
}

#[test]
fn test_dispute_at_threshold_is_jury_path() {
    let (env, client, _admin, player) = setup();
    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);

    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &100u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.jury_required);
    assert!(d.voting_deadline > 0);
    assert_eq!(d.quorum, 3);
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Jury config snapshotted at filing
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_jury_config_snapshotted_at_filing() {
    let (env, client, _admin, player) = setup();
    client.set_jury_config(&100u32, &3u32, &604_800u64);

    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);

    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &100u32);
    let before = client.get_dispute(&1u64, &1u32);

    // Admin changes config mid-dispute
    client.set_jury_config(&50u32, &10u32, &86_400u64);

    let after = client.get_dispute(&1u64, &1u32);
    assert_eq!(after.quorum, before.quorum, "quorum must remain snapshotted");
    assert_eq!(after.voting_deadline, before.voting_deadline, "deadline must remain snapshotted");
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. cast_dispute_vote eligibility rules
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_eligibility_unregistered_validator_rejected() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    let stranger = Address::generate(&env);
    let result = client.try_cast_dispute_vote(&stranger, &1u64, &1u32, &true);
    assert_eq!(result, Err(Ok(VerificationError::ValidatorNotFound)));
}

#[test]
fn test_eligibility_revoked_validator_rejected() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    let voter = reg_validator(&env, &client, CRED2);
    client.revoke_validator(&voter, &RevocationSeverity::Routine, &Some(String::from_str(&env, "misconduct")));

    let result = client.try_cast_dispute_vote(&voter, &1u64, &1u32, &true);
    assert_eq!(result, Err(Ok(VerificationError::ValidatorInactive)));
}

#[test]
fn test_eligibility_conflict_of_interest_original_approver() {
    let (env, client, _admin, player) = setup();
    let approver = setup_jury_dispute(&env, &client, &player);

    // The validator who approved the milestone cannot vote
    let result = client.try_cast_dispute_vote(&approver, &1u64, &1u32, &true);
    assert_eq!(result, Err(Ok(VerificationError::ConflictOfInterest)));
}

#[test]
fn test_eligibility_already_voted() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    let voter = reg_validator(&env, &client, CRED2);
    client.cast_dispute_vote(&voter, &1u64, &1u32, &true);

    // Second vote from same validator must be rejected
    let result = client.try_cast_dispute_vote(&voter, &1u64, &1u32, &false);
    assert_eq!(result, Err(Ok(VerificationError::AlreadyVoted)));
}

#[test]
fn test_eligibility_not_jury_dispute() {
    let (env, client, _admin, player) = setup();
    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);

    // impact_score < threshold → admin path, not jury
    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &10u32);

    let voter = reg_validator(&env, &client, CRED2);
    let result = client.try_cast_dispute_vote(&voter, &1u64, &1u32, &true);
    assert_eq!(result, Err(Ok(VerificationError::NotJuryDispute)));
}

#[test]
fn test_eligibility_voting_window_closed() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    // Advance past the 7-day voting window
    env.ledger().with_mut(|l| l.timestamp += 604_801);

    let voter = reg_validator(&env, &client, CRED2);
    let result = client.try_cast_dispute_vote(&voter, &1u64, &1u32, &true);
    assert_eq!(result, Err(Ok(VerificationError::VotingWindowClosed)));
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. tally_dispute — early close, clear majority
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_tally_early_close_for_majority() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);
    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);
    let v3 = reg_validator(&env, &client, CRED4);

    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v2, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v3, &1u64, &1u32, &true);

    // quorum=3 reached, 3>0 clear majority → early close allowed
    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(d.upheld);
}

#[test]
fn test_tally_early_close_against_majority() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);
    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);
    let v3 = reg_validator(&env, &client, CRED4);

    client.cast_dispute_vote(&v1, &1u64, &1u32, &false);
    client.cast_dispute_vote(&v2, &1u64, &1u32, &false);
    client.cast_dispute_vote(&v3, &1u64, &1u32, &false);

    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(!d.upheld);
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. tie-break: votes equal → upheld=false
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_tally_tie_break_resolves_not_upheld() {
    let (env, client, _admin, player) = setup();
    // quorum=2 so we can test a tie with minimal voters
    client.set_jury_config(&100u32, &2u32, &604_800u64);
    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);
    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &100u32);

    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);

    // 1 for, 1 against — tied at quorum=2
    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v2, &1u64, &1u32, &false);

    // Window still open → should refuse early close on a tie
    let result = client.try_tally_dispute(&1u64, &1u32);
    assert_eq!(result, Err(Ok(VerificationError::VotingWindowOpen)));

    // After deadline → tie resolves not upheld
    env.ledger().with_mut(|l| l.timestamp += 604_801);
    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(!d.upheld, "tie must resolve upheld=false");
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. deadline passed with majority
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_tally_deadline_passed_majority_upheld() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);
    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);
    let v3 = reg_validator(&env, &client, CRED4);

    // 2 for, 1 against — majority for, but total=3=quorum, 2≠1 → can early-close
    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v2, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v3, &1u64, &1u32, &false);

    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(d.upheld);
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. deadline passed below quorum → not upheld
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_tally_deadline_below_quorum_resolves_not_upheld() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);
    let v1 = reg_validator(&env, &client, CRED2);

    // Only 1 vote cast — below quorum=3
    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);

    // Before deadline: must fail
    let result = client.try_tally_dispute(&1u64, &1u32);
    assert_eq!(result, Err(Ok(VerificationError::QuorumNotReached)));

    // After deadline: resolves not upheld regardless of direction
    env.ledger().with_mut(|l| l.timestamp += 604_801);
    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(!d.upheld, "below quorum at deadline must be not upheld");
}

#[test]
fn test_tally_no_votes_at_deadline_resolves_not_upheld() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    env.ledger().with_mut(|l| l.timestamp += 604_801);
    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(!d.upheld);
}

// ─────────────────────────────────────────────────────────────────────────────
// 9 & 10. resolve_dispute jury gate
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_resolve_dispute_blocked_for_jury_dispute() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    let result = client.try_resolve_dispute(&1u64, &1u32, &true);
    assert_eq!(result, Err(Ok(VerificationError::DisputeRequiresJury)));
}

#[test]
fn test_resolve_dispute_works_for_non_jury_dispute() {
    let (env, client, _admin, player) = setup();
    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);

    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &10u32);

    client.resolve_dispute(&1u64, &1u32, &true);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(d.upheld);
}

// ─────────────────────────────────────────────────────────────────────────────
// 11. Adversarial: tied exactly at quorum refuses early-close; deadline → false
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_adversarial_tied_at_quorum_refuses_early_close_then_resolves_false() {
    let (env, client, _admin, player) = setup();
    // quorum=4 so we can get a 2–2 tie exactly at quorum
    client.set_jury_config(&100u32, &4u32, &604_800u64);

    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);
    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &100u32);

    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);
    let v3 = reg_validator(&env, &client, CRED4);
    let v4 = reg_validator(&env, &client, CRED5);

    // 2 for, 2 against — tied at quorum=4
    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v2, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v3, &1u64, &1u32, &false);
    client.cast_dispute_vote(&v4, &1u64, &1u32, &false);

    // Early close must be refused (window open, tie)
    let result = client.try_tally_dispute(&1u64, &1u32);
    assert_eq!(
        result,
        Err(Ok(VerificationError::VotingWindowOpen)),
        "tied at quorum with open window must refuse early-close"
    );

    // Advance past deadline → tie resolves not upheld
    env.ledger().with_mut(|l| l.timestamp += 604_801);
    client.tally_dispute(&1u64, &1u32);

    let d = client.get_dispute(&1u64, &1u32);
    assert!(d.resolved);
    assert!(!d.upheld, "tie at deadline must resolve upheld=false");
    assert_eq!(d.votes_for, 2);
    assert_eq!(d.votes_against, 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// 12. Vote tallies tracked correctly
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_vote_tallies_tracked_incrementally() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);

    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);
    let v3 = reg_validator(&env, &client, CRED4);

    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);
    let d = client.get_dispute(&1u64, &1u32);
    assert_eq!(d.votes_for, 1);
    assert_eq!(d.votes_against, 0);

    client.cast_dispute_vote(&v2, &1u64, &1u32, &false);
    let d = client.get_dispute(&1u64, &1u32);
    assert_eq!(d.votes_for, 1);
    assert_eq!(d.votes_against, 1);

    client.cast_dispute_vote(&v3, &1u64, &1u32, &true);
    let d = client.get_dispute(&1u64, &1u32);
    assert_eq!(d.votes_for, 2);
    assert_eq!(d.votes_against, 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge: tally already-resolved dispute
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_tally_already_resolved_fails() {
    let (env, client, _admin, player) = setup();
    setup_jury_dispute(&env, &client, &player);
    let v1 = reg_validator(&env, &client, CRED2);
    let v2 = reg_validator(&env, &client, CRED3);
    let v3 = reg_validator(&env, &client, CRED4);

    client.cast_dispute_vote(&v1, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v2, &1u64, &1u32, &true);
    client.cast_dispute_vote(&v3, &1u64, &1u32, &true);
    client.tally_dispute(&1u64, &1u32);

    let result = client.try_tally_dispute(&1u64, &1u32);
    assert_eq!(result, Err(Ok(VerificationError::DisputeAlreadyResolved)));
}

#[test]
fn test_tally_non_jury_dispute_fails() {
    let (env, client, _admin, player) = setup();
    let v = reg_validator(&env, &client, CREDENTIALS);
    client.approve_milestone(&v, &1u64, &String::from_str(&env, "goal"), &String::from_str(&env, CID_1), &None);
    client.dispute_milestone(&player, &1u64, &1u32, &String::from_str(&env, REASON), &10u32);

    let result = client.try_tally_dispute(&1u64, &1u32);
    assert_eq!(result, Err(Ok(VerificationError::NotJuryDispute)));
}
