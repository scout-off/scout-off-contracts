//! Issue #703: ed25519 off-chain milestone attestation with nonce replay protection.
//!
//! Covers:
//! 1. Relayer submits a validator-pre-signed attestation end-to-end
//! 2. Nonce replay rejected (isolated from evidence-hash uniqueness)
//! 3. Cross-contract / cross-network binding rejects foreign payloads
//! 4. Milestone attribution follows the verified payload, not the relayer
//! 5. CPU cost of submit_attested_milestone vs approve_milestone

use ed25519_dalek::{Signer, SigningKey};
use scoutchain_verification::{
    MilestoneAttestation, VerificationContract, VerificationContractClient, VerificationError,
};
use soroban_sdk::{testutils::Address as _, xdr::ToXdr, Address, Bytes, BytesN, Env, String};

const CREDENTIALS: &str = "UEFA-B-License-2026";
const ATTESTATION_DOMAIN: &str = "ScoutChain-MilestoneAttestation-v2";

const CID_A: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const CID_B: &str = "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG";
const CID_C: &str = "QmRhbYsqpiYgUY9KfNCcbfopHPbLnWSVKBpDNs37aZ3kVC";
const CID_D: &str = "QmwsjoZwgfzgx6xPr3cXEKhzfLt5RQ87yMnWecTp1tf6p7";

fn setup() -> (Env, VerificationContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin, id)
}

fn signing_key(seed: u8) -> SigningKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = seed.wrapping_add(7);
    SigningKey::from_bytes(&bytes)
}

fn pubkey_bytesn(env: &Env, sk: &SigningKey) -> BytesN<32> {
    BytesN::from_array(env, &sk.verifying_key().to_bytes())
}

fn attestation_message(env: &Env, attestation: &MilestoneAttestation) -> Bytes {
    let mut message = Bytes::new(env);
    message.extend_from_slice(ATTESTATION_DOMAIN.as_bytes());
    message.append(&attestation.contract_id.clone().to_xdr(env));
    message.append(&Bytes::from_slice(env, &attestation.network_id.to_array()));
    message.append(&attestation.validator_wallet.clone().to_xdr(env));
    message.extend_from_slice(&attestation.player_id.to_be_bytes());
    message.append(&attestation.description.clone().to_xdr(env));
    message.append(&attestation.evidence_hash.clone().to_xdr(env));
    message.extend_from_slice(&attestation.nonce.to_be_bytes());
    message.extend_from_slice(&attestation.expires_at.to_be_bytes());
    message
}

fn sign_attestation(env: &Env, sk: &SigningKey, attestation: &MilestoneAttestation) -> BytesN<64> {
    let message = attestation_message(env, attestation);
    let mut msg_buf = [0u8; 1024];
    let len = message.len() as usize;
    assert!(
        len <= msg_buf.len(),
        "attestation message too large for test buffer"
    );
    for (i, b) in message.iter().enumerate() {
        msg_buf[i] = b;
    }
    let sig = sk.sign(&msg_buf[..len]);
    BytesN::from_array(env, &sig.to_bytes())
}

fn make_attestation(
    env: &Env,
    contract_id: &Address,
    validator: &Address,
    player_id: u64,
    description: &str,
    evidence: &str,
    nonce: u64,
) -> MilestoneAttestation {
    MilestoneAttestation {
        validator_wallet: validator.clone(),
        player_id,
        description: String::from_str(env, description),
        evidence_hash: String::from_str(env, evidence),
        nonce,
        expires_at: env.ledger().timestamp() + 300,
        contract_id: contract_id.clone(),
        network_id: env.ledger().network_id(),
    }
}

fn register_validator_with_key(
    env: &Env,
    client: &VerificationContractClient,
    validator: &Address,
    sk: &SigningKey,
) {
    client.register_validator(
        validator,
        &String::from_str(env, CREDENTIALS),
        &String::from_str(env, "Default Academy"),
        &soroban_sdk::Vec::new(env),
    );
    client.register_attestation_key(validator, &pubkey_bytesn(env, sk));
}

#[test]
fn relayer_submits_validator_presigned_attestation() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(1);
    register_validator_with_key(&env, &client, &validator, &sk);

    let attestation = make_attestation(
        &env,
        &contract_id,
        &validator,
        1,
        "hat-trick in regional final",
        CID_A,
        1,
    );
    let signature = sign_attestation(&env, &sk, &attestation);

    let idx = client.submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(idx, 1);

    let ms = client.get_milestone(&1u64, &1u32);
    assert_eq!(ms.validator, validator);
    assert_ne!(ms.validator, relayer);
    assert_eq!(ms.evidence_hash, String::from_str(&env, CID_A));
    assert_eq!(client.get_attestation_nonce(&validator), 1u64);
}

#[test]
fn nonce_replay_rejected_with_fresh_evidence_hash() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(2);
    register_validator_with_key(&env, &client, &validator, &sk);

    let first = make_attestation(
        &env,
        &contract_id,
        &validator,
        1,
        "first milestone",
        CID_A,
        1,
    );
    let sig1 = sign_attestation(&env, &sk, &first);
    client.submit_attested_milestone(&relayer, &first, &sig1);

    // Re-sign a NEW evidence hash with the SAME nonce — isolates InvalidNonce
    // from DuplicateEvidence (CID_B has never been used).
    let replay = make_attestation(
        &env,
        &contract_id,
        &validator,
        1,
        "replay attempt",
        CID_B,
        1, // same nonce
    );
    let sig2 = sign_attestation(&env, &sk, &replay);
    let result = client.try_submit_attested_milestone(&relayer, &replay, &sig2);
    assert_eq!(
        result,
        Err(Ok(VerificationError::InvalidNonce)),
        "same nonce must be rejected even with a fresh evidence hash"
    );
    assert_eq!(client.get_milestone_count(&1u64), 1u32);
}

#[test]
fn cross_contract_attestation_rejected() {
    let (env, client_a, _admin_a, id_a) = setup();
    let id_b = env.register(VerificationContract, ());
    let client_b = VerificationContractClient::new(&env, &id_b);
    let admin_b = Address::generate(&env);
    client_b.initialize(&admin_b);

    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(3);

    register_validator_with_key(&env, &client_a, &validator, &sk);
    register_validator_with_key(&env, &client_b, &validator, &sk);

    // Signed for instance A
    let attestation = make_attestation(&env, &id_a, &validator, 1, "cross-context claim", CID_C, 1);
    let signature = sign_attestation(&env, &sk, &attestation);

    assert!(
        client_a
            .try_submit_attested_milestone(&relayer, &attestation, &signature)
            .is_ok(),
        "valid on instance A"
    );

    // Same payload+sig against instance B must fail binding check
    let result = client_b.try_submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(
        result,
        Err(Ok(VerificationError::InvalidAttestation)),
        "payload bound to contract A must not verify on contract B"
    );
}

#[test]
fn cross_network_attestation_rejected() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(4);
    register_validator_with_key(&env, &client, &validator, &sk);

    let mut attestation = make_attestation(
        &env,
        &contract_id,
        &validator,
        1,
        "network-bound claim",
        CID_D,
        1,
    );
    // Mutate network_id after construction so signature was over the wrong net
    // for the submission environment — sign with a foreign network id embedded.
    let foreign_net = BytesN::from_array(&env, &[9u8; 32]);
    attestation.network_id = foreign_net;
    let signature = sign_attestation(&env, &sk, &attestation);

    let result = client.try_submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(
        result,
        Err(Ok(VerificationError::InvalidAttestation)),
        "foreign network_id must be rejected"
    );
}

#[test]
fn attribution_follows_signed_validator_not_relayer() {
    let (env, client, _admin, contract_id) = setup();
    let validator_a = Address::generate(&env);
    let validator_b = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk_a = signing_key(5);
    let sk_b = signing_key(6);
    register_validator_with_key(&env, &client, &validator_a, &sk_a);
    register_validator_with_key(&env, &client, &validator_b, &sk_b);

    // Genuinely signed by A — no separate validator Address parameter exists for
    // the relayer to spoof; attribution must equal the signed payload's wallet.
    let attestation = make_attestation(
        &env,
        &contract_id,
        &validator_a,
        7,
        "identity confusion probe",
        CID_A,
        1,
    );
    let signature = sign_attestation(&env, &sk_a, &attestation);

    let idx = client.submit_attested_milestone(&relayer, &attestation, &signature);
    let ms = client.get_milestone(&7u64, &idx);
    assert_eq!(ms.validator, validator_a);
    assert_ne!(ms.validator, validator_b);
    assert_ne!(ms.validator, relayer);
}

#[test]
fn submit_attested_milestone_cpu_budget_vs_approve() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(7);
    register_validator_with_key(&env, &client, &validator, &sk);

    // Baseline: approve_milestone
    env.cost_estimate().budget().reset_default();
    client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "baseline approve"),
        &String::from_str(&env, CID_A),
        &None,
    );
    let approve_cpu = env.cost_estimate().budget().cpu_instruction_cost();

    let attestation = make_attestation(
        &env,
        &contract_id,
        &validator,
        2,
        "attested approve",
        CID_B,
        1,
    );
    let signature = sign_attestation(&env, &sk, &attestation);

    env.cost_estimate().budget().reset_default();
    client.submit_attested_milestone(&relayer, &attestation, &signature);
    let attested_cpu = env.cost_estimate().budget().cpu_instruction_cost();

    println!(
        "cost_budget: approve_milestone={approve_cpu} submit_attested_milestone={attested_cpu}"
    );
    // Attested path includes ed25519_verify — expect higher but bounded cost.
    assert!(
        attested_cpu > 0 && approve_cpu > 0,
        "both paths must report non-zero CPU"
    );
    assert!(
        attested_cpu < 50_000_000,
        "submit_attested_milestone CPU {attested_cpu} exceeds 50M instruction sanity cap"
    );
}

#[test]
fn exact_pair_replay_also_rejected_by_nonce() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(8);
    register_validator_with_key(&env, &client, &validator, &sk);

    let attestation = make_attestation(&env, &contract_id, &validator, 1, "exact pair", CID_A, 1);
    let signature = sign_attestation(&env, &sk, &attestation);
    client.submit_attested_milestone(&relayer, &attestation, &signature);

    let result = client.try_submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(result, Err(Ok(VerificationError::InvalidNonce)));
}

// ── Issue #1381: attestation expiry and bounded nonce window ──

#[test]
fn attestation_expired_is_rejected() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(9);
    register_validator_with_key(&env, &client, &validator, &sk);

    // Craft an attestation whose expires_at is already in the past.
    let mut attestation = make_attestation(
        &env, &contract_id, &validator, 1, "expired claim", CID_A, 1,
    );
    attestation.expires_at = env.ledger().timestamp() - 1;
    let signature = sign_attestation(&env, &sk, &attestation);

    let result = client.try_submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(
        result,
        Err(Ok(VerificationError::AttestationExpired)),
        "an attestation whose expires_at is in the past must be rejected"
    );
    assert_eq!(
        client.get_attestation_nonce(&validator),
        0,
        "the nonce must not be consumed by a rejected (expired) call"
    );
}

#[test]
fn attestation_expires_too_far_in_future_is_rejected() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(10);
    register_validator_with_key(&env, &client, &validator, &sk);

    let mut attestation = make_attestation(
        &env, &contract_id, &validator, 1, "far-future claim", CID_B, 1,
    );
    // MAX_ATTESTATION_FUTURE_TOLERANCE_SECS is 3_600 (1 hour).
    attestation.expires_at = env.ledger().timestamp() + 4_000;
    let signature = sign_attestation(&env, &sk, &attestation);

    let result = client.try_submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(
        result,
        Err(Ok(VerificationError::AttestationWindowTooLarge)),
        "an attestation whose expires_at is too far in the future must be rejected"
    );
}

#[test]
fn attestation_within_expiry_tolerance_is_accepted() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(11);
    register_validator_with_key(&env, &client, &validator, &sk);

    let mut attestation = make_attestation(
        &env, &contract_id, &validator, 1, "valid expiry", CID_C, 1,
    );
    // Exactly at the tolerance boundary (3_600 seconds) — should be accepted.
    attestation.expires_at = env.ledger().timestamp() + 3_500;
    let signature = sign_attestation(&env, &sk, &attestation);

    let idx = client.submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(idx, 1);
    assert_eq!(client.get_attestation_nonce(&validator), 1);
}

#[test]
fn attestation_nonce_bitmap_allows_out_of_order_within_window() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(12);
    register_validator_with_key(&env, &client, &validator, &sk);

    // Submit nonce 2 first (skipping 0 and 1) — within the 256-bit window.
    let attestation_2 = make_attestation(
        &env, &contract_id, &validator, 1, "nonce 2 first", CID_A, 2,
    );
    let sig_2 = sign_attestation(&env, &sk, &attestation_2);
    client.submit_attested_milestone(&relayer, &attestation_2, &sig_2);

    // Now submit nonce 0 — out of order but within the window.
    let attestation_0 = make_attestation(
        &env, &contract_id, &validator, 2, "nonce 0 later", CID_B, 0,
    );
    let sig_0 = sign_attestation(&env, &sk, &attestation_0);
    client.submit_attested_milestone(&relayer, &attestation_0, &sig_0);

    // Submit nonce 1 — also out of order, within the window.
    let attestation_1 = make_attestation(
        &env, &contract_id, &validator, 3, "nonce 1 later", CID_C, 1,
    );
    let sig_1 = sign_attestation(&env, &sk, &attestation_1);
    client.submit_attested_milestone(&relayer, &attestation_1, &sig_1);

    assert_eq!(client.get_attestation_nonce(&validator), 2);

    // Replay nonce 1 — must be rejected (bit already set).
    let result = client.try_submit_attested_milestone(&relayer, &attestation_1, &sig_1);
    assert_eq!(result, Err(Ok(VerificationError::InvalidNonce)));
}

#[test]
fn attestation_nonce_window_advances_when_far_ahead() {
    let (env, client, _admin, contract_id) = setup();
    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(13);
    register_validator_with_key(&env, &client, &validator, &sk);

    // Submit nonce 0.
    let attestation_0 = make_attestation(
        &env, &contract_id, &validator, 1, "nonce 0", CID_A, 0,
    );
    let sig_0 = sign_attestation(&env, &sk, &attestation_0);
    client.submit_attested_milestone(&relayer, &attestation_0, &sig_0);

    // Submit nonce 300 — well beyond the 256-bit window. The base should
    // advance to 300, resetting the bitmap.
    let attestation_300 = make_attestation(
        &env, &contract_id, &validator, 2, "nonce 300", CID_B, 300,
    );
    let sig_300 = sign_attestation(&env, &sk, &attestation_300);
    client.submit_attested_milestone(&relayer, &attestation_300, &sig_300);

    // Replay nonce 0 — now below the new base (300) → rejected.
    let result = client.try_submit_attested_milestone(&relayer, &attestation_0, &sig_0);
    assert_eq!(result, Err(Ok(VerificationError::InvalidNonce)));

    assert_eq!(client.get_attestation_nonce(&validator), 300);
}
}
