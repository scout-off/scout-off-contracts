//! Hardening follow-up to #1026's k-of-n threshold milestone attestation
//! (issue #701).
//!
//! Covers three defects found while auditing the merged implementation
//! against the issue's threat model:
//!
//! 1. `submit_attested_milestone` (the off-chain ed25519-relay commit path,
//!    issue #703) never checked the same `ThresholdModeRequiresAttestation`
//!    gate `approve_milestone` does. It calls the identical
//!    `commit_approved_milestone` on the strength of exactly one signature,
//!    so once an operator opts into k-of-n mode specifically to close the
//!    single-validator trust gap, this sibling function remained a fully
//!    open, ungated bypass of the entire mechanism.
//! 2. `has_attested` is documented as reporting a "not-yet-expired" vote, but
//!    round-bumping on window expiry is lazy (it only happens inside the
//!    next `attest_milestone` call for that claim) — so it returned `true`
//!    for a vote that had already practically expired, contradicting its own
//!    documented contract.
//! 3. The per-validator `MAX_PENDING_VOTES_PER_VALIDATOR` bookkeeping
//!    double-counted a validator's own claim when that validator's revote
//!    was itself what triggered the claim's lazy round-bump, prematurely
//!    consuming one extra slot of that validator's 25-claim budget and
//!    incorrectly rejecting a subsequent legitimate vote with
//!    `TooManyPendingVotes`.

use ed25519_dalek::{Signer, SigningKey};
use scoutchain_verification::{
    AttestationStatus, MilestoneAttestation, VerificationContract, VerificationContractClient,
    VerificationError,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    testutils::{MockAuth, MockAuthInvoke},
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, IntoVal, String,
};

const CREDENTIALS: &str = "UEFA-B-License-2026";
const ATTESTATION_DOMAIN: &str = "ScoutChain-MilestoneAttestation-v1";
const DEFAULT_VOTING_WINDOW_SECS: u64 = 1_209_600; // 14 days — must match lib.rs default

fn setup() -> (Env, VerificationContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin, id)
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
/// base58btc characters) for a given seed — mirrors the helper in
/// `threshold_milestone_attestation.rs`.
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

// ── #1: submit_attested_milestone must honor k-of-n threshold mode ──

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
fn submit_attested_milestone_is_closed_once_threshold_mode_is_configured() {
    let (env, client, _admin, contract_id) = setup();
    client.set_milestone_threshold(&2u32);

    let validator = Address::generate(&env);
    let relayer = Address::generate(&env);
    let sk = signing_key(1);
    register_validator_with_key(&env, &client, &validator, &sk);

    let attestation = MilestoneAttestation {
        validator_wallet: validator.clone(),
        player_id: 1,
        description: String::from_str(&env, "hat-trick in regional final"),
        evidence_hash: cid(&env, 100),
        nonce: 1,
        contract_id: contract_id.clone(),
        network_id: env.ledger().network_id(),
    };
    let signature = sign_attestation(&env, &sk, &attestation);

    let result = client.try_submit_attested_milestone(&relayer, &attestation, &signature);
    assert_eq!(
        result,
        Err(Ok(VerificationError::ThresholdModeRequiresAttestation)),
        "a single off-chain-signed attestation must not bypass a configured k-of-n threshold \
         — submit_attested_milestone calls the same commit path as approve_milestone on the \
         strength of exactly one signature and must be gated identically"
    );
    assert_eq!(
        client.get_milestone_count(&1u64),
        0,
        "no milestone may have been committed, and no advance_level cross-call fired"
    );
    assert_eq!(
        client.get_attestation_nonce(&validator),
        0,
        "the nonce must not be consumed by a rejected call"
    );

    // attest_milestone remains the only path to commit once threshold mode
    // is configured, exactly as it does for approve_milestone.
    let via_attest = attest_as(
        &env,
        &client,
        &validator,
        1,
        &String::from_str(&env, "hat-trick in regional final"),
        &cid(&env, 100),
    );
    assert_eq!(via_attest, AttestationStatus::Pending(1));
}

// ── #2: has_attested must honor its own "not-yet-expired" contract ──

#[test]
fn has_attested_returns_false_once_window_has_expired_even_before_the_next_vote_rolls_the_round() {
    let (env, client, _admin, _id) = setup();
    client.set_milestone_threshold(&3u32);

    let v1 = register_validator(&env, &client);
    let player_id = 42u64;
    let description = String::from_str(&env, "identity confirmed by academy");
    let evidence = cid(&env, 200);

    attest_as(&env, &client, &v1, player_id, &description, &evidence);
    assert!(client.has_attested(&player_id, &evidence, &v1));

    // Advance past the voting window. Nobody else has touched this claim, so
    // its `round` field is still the stale pre-expiry round — the bump only
    // happens lazily, inside the next `attest_milestone` call.
    env.ledger().with_mut(|l| {
        l.timestamp += DEFAULT_VOTING_WINDOW_SECS + 1;
    });
    assert!(client.is_attestation_window_expired(&player_id, &evidence));

    assert!(
        !client.has_attested(&player_id, &evidence, &v1),
        "has_attested is documented as reporting a 'not-yet-expired' vote — it must not still \
         report true for a vote that has already exceeded the voting window, even though the \
         claim's round has not yet been formally rolled over by a fresh vote"
    );
}

// ── #3: pending-vote cap must not double-count a validator's own ──
// ── self-triggered round expiry on the same claim ──

#[test]
fn pending_vote_cap_does_not_double_count_a_validators_own_self_triggered_expiry() {
    let (env, client, _admin, _id) = setup();
    // threshold=2 so a lone vote never auto-commits (which would remove the
    // claim and short-circuit the scenario) — every claim in this test stays
    // open on exactly one vote from `target`.
    client.set_milestone_threshold(&2u32);

    let target = register_validator(&env, &client);

    // Open 24 distinct claims with exactly one vote each from `target`. This
    // is the maximum that can be legitimately open while still leaving room
    // for one more (the cap is 25).
    for seed in 0..24u32 {
        let evidence = cid(&env, seed);
        let description = String::from_str(&env, "filler claim");
        let r = attest_as(&env, &client, &target, seed as u64, &description, &evidence);
        assert_eq!(r, AttestationStatus::Pending(1));
    }

    // A 25th claim: target votes once, then the round expires with nobody
    // else having touched it.
    let target_evidence = cid(&env, 999);
    let target_description = String::from_str(&env, "the 25th claim");
    let first = attest_as(
        &env,
        &client,
        &target,
        999,
        &target_description,
        &target_evidence,
    );
    assert_eq!(first, AttestationStatus::Pending(1));

    env.ledger().with_mut(|l| {
        l.timestamp += DEFAULT_VOTING_WINDOW_SECS + 1;
    });

    // `target` revotes on their own now-expired 25th claim. This is the
    // exact scenario that triggers the claim's own lazy round-bump inside
    // this same call. Before the fix, the pruning loop re-read the
    // not-yet-persisted (still pre-bump) claim from storage, incorrectly
    // kept the stale round-0 ref as "live" alongside the fresh round-1 ref
    // pushed at the end of the call, and the cap check — seeing 24 filler
    // refs plus the wrongly-retained stale ref, i.e. 25 — rejected this
    // revote with `TooManyPendingVotes`, even though `target` only
    // legitimately has 24 other open claims plus this one (25 total, right
    // at, not over, the cap).
    let revote = attest_as(
        &env,
        &client,
        &target,
        999,
        &target_description,
        &target_evidence,
    );
    assert_eq!(
        revote,
        AttestationStatus::Pending(1),
        "revoting on an expired claim that only this validator had ever voted on must not be \
         rejected as TooManyPendingVotes — the stale pre-expiry ref for this exact claim must \
         not be double-counted alongside the fresh post-expiry ref for the same claim"
    );
}
