//! Adversarial tests for issue #811: idempotency and all-or-nothing revert
//! guarantee for `approve_milestone`.
//!
//! `approve_milestone`'s dedup key is `EvidenceUsed(evidence_hash)` — a
//! retry that reuses an already-committed evidence hash is rejected with
//! `DuplicateEvidence`, so a milestone can never be double-committed. Because
//! a failed attempt (e.g. `ProgressCallFailed`) reverts the *entire*
//! transaction, the dedup key is rolled back with it and the retry applies
//! exactly once. These tests prove both properties.

use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

const VALID_CID: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const VALID_CID_2: &str = "QmvwxyzABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdef";

#[test]
fn test_approve_milestone_progress_call_failed_reverts_all_state() {
    let env = Env::default();
    env.mock_all_auths();

    let verification_id = env.register(VerificationContract, ());
    let verification_client = VerificationContractClient::new(&env, &verification_id);

    let progress_id = env.register(scoutchain_progress::ProgressContract, ());
    let progress_client = scoutchain_progress::ProgressContractClient::new(&env, &progress_id);

    let admin = Address::generate(&env);
    let validator = Address::generate(&env);
    let player_id = 1u64;

    verification_client.initialize(&admin);
    verification_client.set_progress_contract(&progress_id);
    verification_client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License-2026"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::Vec::new(&env));

    progress_client.initialize(&admin);

    // Call approve_milestone — the progress contract is initialized but
    // has no registration contract linked, so advance_level will fail with
    // a cross-contract error. This simulates a misconfigured deployment.
    let result = verification_client.try_approve_milestone(
        &validator,
        &player_id,
        &String::from_str(&env, "hat-trick"),
        &String::from_str(&env, VALID_CID),
        &None,
    );

    // The call should fail with ProgressCallFailed
    assert!(
        result.is_err(),
        "approve_milestone should fail when progress.advance_level fails"
    );

    // Verify NO partial state was persisted (the whole transaction reverted):
    // 1. Milestone counter should still be 0
    let counter: u32 = env.as_contract(&verification_id, || {
        env.storage()
            .persistent()
            .get(&scoutchain_verification::DataKey::MilestoneCounter(
                player_id,
            ))
            .unwrap_or(0)
    });
    assert_eq!(
        counter, 0,
        "Milestone counter must not be incremented on reverted ProgressCallFailed"
    );

    // 2. No milestone should exist
    let milestone = env.as_contract(&verification_id, || {
        env.storage()
            .persistent()
            .get::<scoutchain_verification::DataKey, scoutchain_verification::Milestone>(
                &scoutchain_verification::DataKey::Milestone(player_id, 1),
            )
    });
    assert!(
        milestone.is_none(),
        "Milestone must not be persisted on reverted ProgressCallFailed"
    );

    // 3. Evidence hash must not be marked as used
    let evidence_used = env.as_contract(&verification_id, || {
        env.storage()
            .persistent()
            .has(&scoutchain_verification::DataKey::EvidenceUsed(
                String::from_str(&env, VALID_CID),
            ))
    });
    assert!(
        !evidence_used,
        "Evidence hash must not be marked as used on reverted ProgressCallFailed"
    );
}

#[test]
fn test_approve_milestone_duplicate_evidence_prevents_double_commit_on_retry() {
    let env = Env::default();
    env.mock_all_auths();

    let verification_id = env.register(VerificationContract, ());
    let verification_client = VerificationContractClient::new(&env, &verification_id);

    let admin = Address::generate(&env);
    let validator = Address::generate(&env);
    let player_id = 1u64;

    verification_client.initialize(&admin);
    verification_client.register_validator(&validator, &String::from_str(&env, "UEFA-B-License-2026"), &String::from_str(&env, "Default Academy"), &String::from_str(&env, "Default Region"), &soroban_sdk::Vec::new(&env));

    // First approval with a fresh evidence hash commits milestone #1.
    let result = verification_client.try_approve_milestone(
        &validator,
        &player_id,
        &String::from_str(&env, "hat-trick"),
        &String::from_str(&env, VALID_CID),
        &None,
    );
    assert!(
        result.is_ok(),
        "First approve_milestone with a fresh evidence hash should succeed"
    );
    assert_eq!(result.unwrap().unwrap(), 1);
    assert_eq!(verification_client.get_milestone_count(&player_id), 1);

    // Retry with the *same* evidence hash (e.g. an RPC-level retry) is
    // rejected by the EvidenceUsed dedup key — the milestone is not
    // double-committed and the counter is not advanced again.
    let retry = verification_client.try_approve_milestone(
        &validator,
        &player_id,
        &String::from_str(&env, "hat-trick (retry)"),
        &String::from_str(&env, VALID_CID),
        &None,
    );
    assert_eq!(
        retry,
        Err(Ok(
            scoutchain_verification::VerificationError::DuplicateEvidence
        )),
        "Retry reusing the same evidence hash must be rejected as a duplicate"
    );
    assert_eq!(
        verification_client.get_milestone_count(&player_id),
        1,
        "Milestone counter must not advance on a duplicate-evidence retry"
    );

    // A genuinely new evidence hash still commits the next milestone.
    let second = verification_client.try_approve_milestone(
        &validator,
        &player_id,
        &String::from_str(&env, "speed test passed"),
        &String::from_str(&env, VALID_CID_2),
        &None,
    );
    assert!(
        second.is_ok(),
        "Fresh evidence hash should commit milestone #2"
    );
    assert_eq!(second.unwrap().unwrap(), 2);
}
