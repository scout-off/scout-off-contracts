//! k-of-n threshold milestone attestation.
//!
//! `approve_milestone` used to commit a milestone and cross-call
//! `progress.advance_level` on the strength of exactly one validator's
//! `require_auth()`. `attest_milestone` replaces that single-signature trust
//! model with an on-chain accumulation pattern: independent, asynchronous
//! votes from distinct active validators accrue in bounded storage until a
//! configurable `threshold` is reached, only then committing the milestone.
//!
//! Covers:
//! 1. Sub-threshold votes do not commit; the threshold-th vote commits exactly once
//! 2. A duplicate vote from the same validator is rejected distinctly from a fresh vote
//! 3. `revoke_validator` retroactively invalidates a still-pending vote
//! 4. A voting-window expiry resets the tally (round-based) instead of leaking storage
//! 5. `approve_milestone`'s single-signature fast path is closed once threshold > 1
//! 6. CPU-instruction cost of the threshold-reaching call does not scale with vote count

use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{
    AttestationStatus, DataKey, RegPlayerProfile, RegPlayerVitals, RevocationSeverity,
    VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    testutils::{MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, String,
};

/// Mirrors the contract's private `MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR`.
const MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR: u32 = 5;

const CREDENTIALS: &str = "UEFA-B-License-2026";
const DEFAULT_VOTING_WINDOW_SECS: u64 = 1_209_600; // 14 days — must match lib.rs default

fn setup() -> (Env, VerificationContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin)
}

fn register_validator(env: &Env, client: &VerificationContractClient) -> Address {
    let wallet = Address::generate(env);
    client.register_validator(
        &wallet,
        &String::from_str(env, CREDENTIALS),
        &String::from_str(env, "Default Academy"),
        &soroban_sdk::Vec::new(env),
    );
    wallet
}

/// Deterministically build a distinct, valid 46-character CIDv0 (`Qm` + 44
/// base58btc characters) for a given seed. `validate_cid` requires exact
/// length and charset, so hand-typing dozens of literals for the scaling
/// test is impractical — this generates as many distinct evidence hashes as
/// needed.
fn cid(env: &Env, seed: u32) -> String {
    const CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut s = std::string::String::from("Qm");
    let mut n = seed.wrapping_add(1);
    for _ in 0..44 {
        let idx = (n % CHARS.len() as u32) as usize;
        s.push(CHARS[idx] as char);
        n = n / (CHARS.len() as u32) + seed.wrapping_add(7);
    }
    String::from_str(env, &s)
}

/// Scope `env.mock_auths()` to exactly this validator's `attest_milestone`
/// invocation — proves `require_auth()` is checked per-call against the
/// specific `validator_wallet` argument, not bypassed by a blanket
/// `mock_all_auths()`. This narrows the authorization mode for whatever call
/// comes next, so any admin-authorized call made afterward in the same test
/// must re-arm `env.mock_all_auths()` first.
fn scope_auth_to(
    env: &Env,
    client: &VerificationContractClient,
    validator: &Address,
    player_id: u64,
    description: &String,
    evidence_hash: &String,
) {
    env.mock_auths(&[MockAuth {
        address: validator,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "attest_milestone",
            args: (
                validator.clone(),
                player_id,
                description.clone(),
                evidence_hash.clone(),
            )
                .into_val(env),
            sub_invokes: &[],
        },
    }]);
}

/// Cast a vote expected to succeed.
fn attest_as(
    env: &Env,
    client: &VerificationContractClient,
    validator: &Address,
    player_id: u64,
    description: &String,
    evidence_hash: &String,
) -> AttestationStatus {
    scope_auth_to(
        env,
        client,
        validator,
        player_id,
        description,
        evidence_hash,
    );
    client.attest_milestone(validator, &player_id, description, evidence_hash)
}

#[test]
fn sub_threshold_votes_do_not_commit_threshold_vote_commits_once() {
    let (env, client, _admin) = setup();

    let v1 = register_validator(&env, &client);
    let v2 = register_validator(&env, &client);
    let v3 = register_validator(&env, &client);
    client.set_milestone_threshold(&3u32);
    let player_id = 1u64;
    let description = String::from_str(&env, "hat-trick in regional final");
    let evidence = cid(&env, 1);

    let r1 = attest_as(&env, &client, &v1, player_id, &description, &evidence);
    assert_eq!(r1, AttestationStatus::Pending(1));
    assert_eq!(client.get_milestone_count(&player_id), 0);
    let claim = client.get_pending_claim(&player_id, &evidence).unwrap();
    assert_eq!(claim.vote_count, 1);
    assert_eq!(claim.threshold, 3);

    let r2 = attest_as(&env, &client, &v2, player_id, &description, &evidence);
    assert_eq!(r2, AttestationStatus::Pending(2));
    assert_eq!(
        client.get_milestone_count(&player_id),
        0,
        "still sub-threshold — no commitment, no advance_level cross-call"
    );

    let r3 = attest_as(&env, &client, &v3, player_id, &description, &evidence);
    match r3 {
        AttestationStatus::Committed(index) => {
            assert_eq!(index, 1);
            let ms = client.get_milestone(&player_id, &index);
            assert_eq!(
                ms.validator, v3,
                "attribution follows the threshold-reaching vote"
            );
            assert_eq!(ms.evidence_hash, evidence);
        }
        other => panic!("expected Committed after the 3rd distinct vote, got {other:?}"),
    }
    assert_eq!(
        client.get_milestone_count(&player_id),
        1,
        "commit fired exactly once"
    );
    assert!(
        client.get_pending_claim(&player_id, &evidence).is_none(),
        "pending accumulator is cleared once committed"
    );
}

#[test]
fn duplicate_attestation_is_rejected_distinctly_from_a_first_time_vote() {
    let (env, client, _admin) = setup();

    let v1 = register_validator(&env, &client);
    let _v2 = register_validator(&env, &client);
    client.set_milestone_threshold(&2u32);
    let player_id = 2u64;
    let description = String::from_str(&env, "top speed 32km/h");
    let evidence = cid(&env, 2);

    let first = attest_as(&env, &client, &v1, player_id, &description, &evidence);
    assert_eq!(first, AttestationStatus::Pending(1));

    scope_auth_to(&env, &client, &v1, player_id, &description, &evidence);
    let second = client.try_attest_milestone(&v1, &player_id, &description, &evidence);
    assert_eq!(
        second,
        Err(Ok(VerificationError::DuplicateAttestation)),
        "a second vote from the same validator must be rejected, not silently no-op like a fresh vote"
    );

    let claim = client.get_pending_claim(&player_id, &evidence).unwrap();
    assert_eq!(
        claim.vote_count, 1,
        "the duplicate must not have counted a second time"
    );
}

#[test]
fn revoke_validator_retroactively_invalidates_a_pending_vote() {
    let (env, client, _admin) = setup();

    let v1 = register_validator(&env, &client);
    let v2 = register_validator(&env, &client);
    let v3 = register_validator(&env, &client);
    let v4 = register_validator(&env, &client);
    client.set_milestone_threshold(&3u32);
    let player_id = 3u64;
    let description = String::from_str(&env, "identity confirmed by academy");
    let evidence = cid(&env, 3);

    let r = attest_as(&env, &client, &v1, player_id, &description, &evidence);
    assert_eq!(r, AttestationStatus::Pending(1));
    assert!(client.has_attested(&player_id, &evidence, &v1));

    // v1 turns out to be compromised — admin revokes for cause. Re-arm
    // blanket auth since the previous call narrowed authorization to v1.
    env.mock_all_auths();
    client.revoke_validator(
        &v1,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "Key compromise suspected")),
    );

    let claim = client
        .get_pending_claim(&player_id, &evidence)
        .expect("claim is still open, just invalidated back to 0 votes");
    assert_eq!(
        claim.vote_count, 0,
        "revocation must retroactively strip the pending vote"
    );
    assert!(
        !client.has_attested(&player_id, &evidence, &v1),
        "the invalidated vote marker must be cleared, not merely ignored"
    );

    // A fresh, still-active validator's vote counts as the FIRST vote, not
    // the second — proving v1's original contribution is truly gone, not
    // just hidden.
    let r2 = attest_as(&env, &client, &v2, player_id, &description, &evidence);
    assert_eq!(
        r2,
        AttestationStatus::Pending(1),
        "must be 1, not 2 — v1's revoked vote must not still be counted"
    );

    let r3 = attest_as(&env, &client, &v3, player_id, &description, &evidence);
    assert_eq!(r3, AttestationStatus::Pending(2));

    let r4 = attest_as(&env, &client, &v4, player_id, &description, &evidence);
    assert!(
        matches!(r4, AttestationStatus::Committed(_)),
        "3 genuinely-active distinct votes (v2, v3, v4) must still be able to reach threshold"
    );
}

#[test]
fn voting_window_expiry_resets_the_tally_instead_of_leaking_storage() {
    let (env, client, _admin) = setup();

    let v1 = register_validator(&env, &client);
    let v2 = register_validator(&env, &client);
    let v3 = register_validator(&env, &client);
    client.set_milestone_threshold(&3u32);
    let player_id = 4u64;
    let description = String::from_str(&env, "academy membership verified");
    let evidence = cid(&env, 4);

    attest_as(&env, &client, &v1, player_id, &description, &evidence);
    attest_as(&env, &client, &v2, player_id, &description, &evidence);
    assert_eq!(
        client
            .get_pending_claim(&player_id, &evidence)
            .unwrap()
            .vote_count,
        2
    );
    assert!(!client.is_attestation_window_expired(&player_id, &evidence));

    // Advance past the configured voting window with the claim still
    // sub-threshold.
    env.ledger().with_mut(|l| {
        l.timestamp += DEFAULT_VOTING_WINDOW_SECS + 1;
    });
    assert!(
        client.is_attestation_window_expired(&player_id, &evidence),
        "storage must be clearly marked expired, not silently unreachable dead state"
    );

    // A vote after expiry starts a fresh round — the 2 pre-expiry votes no
    // longer count, so this is Pending(1), not Committed via 2+1=3.
    let r3 = attest_as(&env, &client, &v3, player_id, &description, &evidence);
    assert_eq!(
        r3,
        AttestationStatus::Pending(1),
        "expired votes must not still count toward threshold"
    );
    let claim = client.get_pending_claim(&player_id, &evidence).unwrap();
    assert_eq!(
        claim.round, 1,
        "expiry must bump the round rather than leaving stale state"
    );
    assert!(!client.is_attestation_window_expired(&player_id, &evidence));

    // v1 and v2 already voted in round 0 — after expiry they must be able to
    // vote again in the new round rather than being permanently locked out
    // by their stale round-0 marker.
    let r1_again = attest_as(&env, &client, &v1, player_id, &description, &evidence);
    assert_eq!(r1_again, AttestationStatus::Pending(2));

    let r2_again = attest_as(&env, &client, &v2, player_id, &description, &evidence);
    assert!(
        matches!(r2_again, AttestationStatus::Committed(_)),
        "3 distinct votes within the new round must still be able to reach threshold"
    );
}

#[test]
fn approve_milestone_still_works_at_default_threshold_one() {
    let (env, client, _admin) = setup();
    let validator = register_validator(&env, &client);

    let idx = client.approve_milestone(
        &validator,
        &5u64,
        &String::from_str(&env, "scored a hat-trick"),
        &cid(&env, 5),
        &None,
    );
    assert_eq!(
        idx, 1,
        "default threshold=1 preserves today's single-signature fast path"
    );
}

#[test]
fn approve_milestone_is_closed_once_threshold_mode_is_configured() {
    let (env, client, _admin) = setup();

    let validator = register_validator(&env, &client);
    let _extra = register_validator(&env, &client);
    client.set_milestone_threshold(&2u32);
    let result = client.try_approve_milestone(
        &validator,
        &6u64,
        &String::from_str(&env, "scored a hat-trick"),
        &cid(&env, 6),
        &None,
    );
    assert_eq!(
        result,
        Err(Ok(VerificationError::ThresholdModeRequiresAttestation)),
        "once k-of-n mode is configured there is no single-signature bypass"
    );

    // attest_milestone remains the only path to commit.
    let via_attest = attest_as(
        &env,
        &client,
        &validator,
        6u64,
        &String::from_str(&env, "scored a hat-trick"),
        &cid(&env, 6),
    );
    assert_eq!(via_attest, AttestationStatus::Pending(1));
}

/// Run one threshold scenario to completion in a fresh, isolated contract
/// instance and return the CPU-instruction cost of the threshold-reaching
/// (committing) call. Each scenario gets its own `Env` so the measurement
/// isn't confounded by ambient ledger state left over from a previous
/// scenario (e.g. a prior scenario's committed milestone growing the shared
/// `GlobalMilestoneIndex`) — the only variable that differs between calls to
/// this helper is the number of distinct validators registered and voting.
fn measure_threshold_reach_cpu(threshold: u32) -> u64 {
    let (env, client, _admin) = setup();

    let validators: std::vec::Vec<Address> = (0..threshold)
        .map(|_| register_validator(&env, &client))
        .collect();
    client.set_milestone_threshold(&threshold);
    let player_id = 1u64;
    let description = String::from_str(&env, "threshold-scaling scenario");
    let evidence = cid(&env, 1);

    for v in &validators[0..(threshold - 1) as usize] {
        attest_as(&env, &client, v, player_id, &description, &evidence);
    }

    env.cost_estimate().budget().reset_default();
    let result = attest_as(
        &env,
        &client,
        &validators[(threshold - 1) as usize],
        player_id,
        &description,
        &evidence,
    );
    assert!(matches!(result, AttestationStatus::Committed(_)));
    env.cost_estimate().budget().cpu_instruction_cost()
}

/// CPU-instruction cost of the threshold-reaching `attest_milestone` call
/// must remain bounded as the number of distinct voters grows.
///
/// Issue #1398 intentionally stores a bounded `voters: Vec<Address>` on the
/// claim (capped by `threshold ≤ MAX_VALIDATORS`) so expired rounds can be
/// pruned in O(threshold). That makes per-vote claim rewrites grow mildly
/// with vote count, so this regression uses modest thresholds (3 → 6) that
/// stay inside the test host footprint limits while still catching an
/// accidental unbounded scan or rewrite.
#[test]
fn cost_attest_milestone_threshold_reach_does_not_scale_with_vote_count() {
    let cpu_lo = measure_threshold_reach_cpu(3);
    let cpu_hi = measure_threshold_reach_cpu(6);

    let delta = cpu_hi.abs_diff(cpu_lo);
    let delta_pct = delta as f64 / cpu_lo as f64 * 100.0;
    println!(
        "cost_budget: attest_milestone threshold-reaching call — threshold=3: {cpu_lo} cpu \
         instructions, threshold=6: {cpu_hi} cpu instructions, delta={delta} ({delta_pct:.1}%)"
    );

    assert!(
        cpu_lo > 0 && cpu_hi > 0,
        "both paths must report non-zero CPU"
    );
    assert!(
        delta_pct < 80.0,
        "attest_milestone cost grew {delta_pct:.1}% going from threshold=3 to threshold=6 \
         ({cpu_lo} -> {cpu_hi} cpu instructions) — suggests an unbounded per-vote scan"
    );
    assert!(
        cpu_hi < 50_000_000,
        "attest_milestone CPU {cpu_hi} exceeds the 50M instruction sanity cap"
    );
}

// ── Attestor-set tests ──────────────────────────────────────────────

/// Committed threshold milestones store the complete attestor set,
/// retrievable via `get_milestone_attestors`.
#[test]
fn committed_milestone_stores_full_attestor_set() {
    let (env, client, _admin) = setup();

    let v1 = register_validator(&env, &client);
    let v2 = register_validator(&env, &client);
    let v3 = register_validator(&env, &client);
    client.set_milestone_threshold(&3u32);
    let player_id = 100u64;
    let description = String::from_str(&env, "attestor set test");
    let evidence = cid(&env, 100);

    attest_as(&env, &client, &v1, player_id, &description, &evidence);
    attest_as(&env, &client, &v2, player_id, &description, &evidence);
    let r3 = attest_as(&env, &client, &v3, player_id, &description, &evidence);
    assert!(matches!(r3, AttestationStatus::Committed(1)));

    let attestors = client.get_milestone_attestors(&player_id, &1u32);
    assert_eq!(attestors.len(), 3, "all 3 attestors must be stored");
    assert!(attestors.contains(&v1), "v1 must be in attestor set");
    assert!(attestors.contains(&v2), "v2 must be in attestor set");
    assert!(attestors.contains(&v3), "v3 must be in attestor set");
}

/// Cascade sweep for any attestor flags the milestone (test with threshold=3,
/// revoke the first voter).
#[test]
fn cascade_sweep_flags_milestone_for_co_attestor() {
    let (env, client, _admin) = setup();

    let v1 = register_validator(&env, &client);
    let v2 = register_validator(&env, &client);
    let v3 = register_validator(&env, &client);
    client.set_milestone_threshold(&3u32);
    let player_id = 200u64;
    let description = String::from_str(&env, "cascade co-attestor test");
    let evidence = cid(&env, 200);

    attest_as(&env, &client, &v1, player_id, &description, &evidence);
    attest_as(&env, &client, &v2, player_id, &description, &evidence);
    let r3 = attest_as(&env, &client, &v3, player_id, &description, &evidence);
    assert!(matches!(r3, AttestationStatus::Committed(1)));

    // Revoke v1 (the first voter, not the primary validator).
    env.mock_all_auths();
    client.revoke_validator(
        &v1,
        &RevocationSeverity::ForCause,
        &Some(String::from_str(&env, "Compromised key")),
    );

    // The milestone must be flagged because v1 is a co-attestor.
    assert!(
        client.is_milestone_flagged(&player_id, &1u32),
        "milestone must be flagged after co-attestor v1 is revoked for cause"
    );
}

/// Per-player-per-validator limits apply to every attestor, not just the
/// primary validator: a co-attestor already at the cap blocks the commit, and
/// a successful commit counts toward every attestor's cap.
#[test]
fn per_validator_cap_applies_to_all_attestors() {
    let (env, client, _admin) = setup();
    let v1 = register_validator(&env, &client);
    let v2 = register_validator(&env, &client);
    client.set_milestone_threshold(&2u32);
    let player_id = 300u64;
    let description = String::from_str(&env, "per-validator cap test");

    // A committed milestone increments both attestors' per-player counts.
    let evidence = cid(&env, 300);
    attest_as(&env, &client, &v1, player_id, &description, &evidence);
    let r2 = attest_as(&env, &client, &v2, player_id, &description, &evidence);
    assert!(matches!(r2, AttestationStatus::Committed(1)));
    let count_for = |v: &Address| -> u32 {
        env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .get(&DataKey::ValidatorPlayerMilestoneCount(
                    v.clone(),
                    player_id,
                ))
                .unwrap_or(0u32)
        })
    };
    assert_eq!(count_for(&v1), 1);
    assert_eq!(count_for(&v2), 1);

    // Push v1 (the co-attestor, not the primary) to the cap.
    env.as_contract(&client.address, || {
        env.storage().persistent().set(
            &DataKey::ValidatorPlayerMilestoneCount(v1.clone(), player_id),
            &MAX_MILESTONES_PER_PLAYER_PER_VALIDATOR,
        );
    });

    // v1 votes first, v2's vote reaches the threshold; the commit is rejected
    // because co-attestor v1 is at the cap, and nothing is committed.
    let evidence2 = cid(&env, 301);
    let r3 = attest_as(&env, &client, &v1, player_id, &description, &evidence2);
    assert_eq!(r3, AttestationStatus::Pending(1));
    scope_auth_to(&env, &client, &v2, player_id, &description, &evidence2);
    let r4 = client.try_attest_milestone(&v2, &player_id, &description, &evidence2);
    assert_eq!(r4, Err(Ok(VerificationError::MilestoneLimitExceeded)));
    assert_eq!(client.get_milestone_count(&player_id), 1);
    assert_eq!(count_for(&v2), 1);
}

#[contracttype]
enum RegStubKey {
    Owner,
}

/// Registration stand-in for `dispute_milestone`'s wallet↔player check: every
/// `player_id` belongs to the wallet stored at init time.
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
                position: String::from_str(&env, "ST"),
                region: String::from_str(&env, "FR"),
                nationality: String::from_str(&env, "ES"),
            },
            ipfs_hashes: soroban_sdk::Vec::new(&env),
            level: ProgressLevel::Unverified,
            registered_at: 0,
            updated_at: 0,
        }
    }
}

/// `cast_dispute_vote` rejects any attestor of the disputed milestone with
/// `ConflictOfInterest` — including a co-attestor from a different academy
/// than the primary approver, so the rejection is not just the affiliation
/// rule.
#[test]
fn dispute_rejects_co_attestor_with_conflict_of_interest() {
    let (env, client, _admin) = setup();
    let player_wallet = Address::generate(&env);
    let reg_id = env.register(RegStub, ());
    RegStubClient::new(&env, &reg_id).initialize(&player_wallet);
    client.set_registration_contract(&reg_id);

    let register = |affiliation: &str| -> Address {
        let wallet = Address::generate(&env);
        client.register_validator(
            &wallet,
            &String::from_str(&env, CREDENTIALS),
            &String::from_str(&env, affiliation),
            &soroban_sdk::Vec::new(&env),
        );
        wallet
    };
    let v1 = register("Academy One");
    let v2 = register("Academy Two");
    client.set_milestone_threshold(&2u32);

    let player_id = 400u64;
    let description = String::from_str(&env, "dispute co-attestor test");
    let evidence = cid(&env, 400);
    attest_as(&env, &client, &v1, player_id, &description, &evidence);
    let r2 = attest_as(&env, &client, &v2, player_id, &description, &evidence);
    assert!(matches!(r2, AttestationStatus::Committed(1)));

    // File a jury dispute on the committed milestone, one second later so
    // v1 and v2 pass the registered-before-filing rule (#1375) and the
    // rejection comes from the co-attestor check.
    env.ledger().with_mut(|l| l.timestamp += 1);
    env.mock_all_auths();
    client.dispute_milestone(
        &player_wallet,
        &player_id,
        &1u32,
        &String::from_str(&env, "dispute reason"),
        &100u32, // impact_score >= jury threshold
    );

    for attestor in [&v1, &v2] {
        assert_eq!(
            client.try_cast_dispute_vote(attestor, &player_id, &1u32, &true),
            Err(Ok(VerificationError::ConflictOfInterest)),
            "attestor {attestor:?} must be rejected with ConflictOfInterest"
        );
    }
}
