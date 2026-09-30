//! Tests for issue #816: pre-authorized migration ticket protocol.
//!
//! Verifies that:
//! 1. A valid pre-authorization is successfully redeemed once
//! 2. A replay/reuse of the same authorization against a second new contract is rejected

use ed25519_dalek::{Signer, SigningKey};
use scoutchain_registration::{
    MigrationAuthorization, MigrationRole, PlayerVitals, RegistrationContract,
    RegistrationContractClient, ScoutChainError,
};
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{
    address_payload::AddressPayload, testutils::Address as _, Address, Bytes, BytesN, Env, String,
    Vec,
};

fn setup() -> (Env, RegistrationContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(RegistrationContract, ());
    let client = RegistrationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn signing_key(seed: u8) -> SigningKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = seed.wrapping_add(7);
    SigningKey::from_bytes(&bytes)
}

/// Construct a G-address whose embedded ed25519 master key equals the verifying
/// key of `sk`.  This mirrors `RegistrationContract::address_to_ed25519_key`,
/// which derives the verification key from the wallet address payload.
fn wallet_from_signing_key(env: &Env, sk: &SigningKey) -> Address {
    let pubkey = BytesN::from_array(env, &sk.verifying_key().to_bytes());
    AddressPayload::AccountIdPublicKeyEd25519(pubkey).to_address(env)
}

fn vitals(env: &Env) -> PlayerVitals {
    PlayerVitals {
        age: 20,
        position: String::from_str(env, "Forward"),
        region: String::from_str(env, "West Africa"),
        nationality: String::from_str(env, "Ghana"),
    }
}

fn ipfs_hashes(env: &Env) -> Vec<String> {
    Vec::from_slice(env, &[String::from_str(env, "QmCID1")])
}

/// Mirrors `RegistrationContract::profile_data_hash` exactly.
fn profile_data_hash(
    env: &Env,
    wallet: &Address,
    vitals: &PlayerVitals,
    ipfs_hashes: &Vec<String>,
    player_id: u64,
    registered_at: u64,
    updated_at: u64,
) -> Bytes {
    let mut buf = Bytes::new(env);
    buf.append(&wallet.to_string().to_bytes());
    buf.extend_from_array(&vitals.age.to_be_bytes());
    buf.append(&vitals.position.to_bytes());
    buf.append(&vitals.region.to_bytes());
    buf.append(&vitals.nationality.to_bytes());
    for i in 0..ipfs_hashes.len() {
        if let Some(h) = ipfs_hashes.get(i) {
            buf.append(&h.to_bytes());
        }
    }
    buf.extend_from_array(&player_id.to_be_bytes());
    buf.extend_from_array(&registered_at.to_be_bytes());
    buf.extend_from_array(&updated_at.to_be_bytes());
    env.crypto().sha256(&buf).into()
}

/// Mirrors `RegistrationContract::migration_message` exactly.
fn migration_message(
    env: &Env,
    wallet: &Address,
    role: &MigrationRole,
    profile_data_hash: &Bytes,
    new_contract_hint: &Address,
    nonce: u64,
    expires_at: u64,
) -> Bytes {
    let mut msg = Bytes::new(env);
    msg.append(&wallet.to_string().to_bytes());
    let role_byte: u8 = match role {
        MigrationRole::Player => 0u8,
        MigrationRole::Scout => 1u8,
    };
    msg.extend_from_array(&[role_byte]);
    msg.append(profile_data_hash);
    msg.append(&new_contract_hint.to_string().to_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&expires_at.to_be_bytes());
    msg
}

fn sign_bytes(env: &Env, sk: &SigningKey, bytes: &Bytes) -> BytesN<64> {
    let mut buf = [0u8; 1024];
    let len = bytes.len() as usize;
    assert!(len <= buf.len(), "message too large for test buffer");
    for (i, b) in bytes.iter().enumerate() {
        buf[i] = b;
    }
    let sig = sk.sign(&buf[..len]);
    BytesN::from_array(env, &sig.to_bytes())
}

fn sign_migration_player(
    env: &Env,
    sk: &SigningKey,
    wallet: &Address,
    player_id: u64,
    new_contract_hint: &Address,
    nonce: u64,
) -> MigrationAuthorization {
    let vitals = vitals(env);
    let hashes = ipfs_hashes(env);
    let hash = profile_data_hash(
        env,
        wallet,
        &vitals,
        &hashes,
        player_id,
        1_700_000_000,
        1_700_000_000,
    );

    let message = migration_message(
        env,
        wallet,
        &MigrationRole::Player,
        &hash,
        new_contract_hint,
        nonce,
        0,
    );
    let signature = sign_bytes(env, sk, &message);

    MigrationAuthorization {
        wallet: wallet.clone(),
        role: MigrationRole::Player,
        profile_data_hash: hash,
        new_contract_hint: new_contract_hint.clone(),
        nonce,
        expires_at: 0,
        signature,
    }
}

#[test]
fn test_valid_migration_authorization_redeemed_once() {
    let (env, client) = setup();

    let sk = signing_key(1);
    let wallet = wallet_from_signing_key(&env, &sk);
    let new_contract_hint = Address::generate(&env);

    let authorization = sign_migration_player(&env, &sk, &wallet, 1, &new_contract_hint, 1);

    let vitals = vitals(&env);
    let hashes = ipfs_hashes(&env);
    let level = ProgressLevel::Unverified;
    let result = client.try_redeem_migration_player(
        &wallet,
        &vitals,
        &hashes,
        &level,
        &1u64,
        &1_700_000_000u64,
        &1_700_000_000u64,
        &authorization,
    );

    assert!(
        result.is_ok(),
        "Valid migration authorization should be redeemed"
    );
}

#[test]
fn test_replay_same_nonce_rejected() {
    let (env, client) = setup();

    let sk = signing_key(2);
    let wallet = wallet_from_signing_key(&env, &sk);
    let new_contract_hint = Address::generate(&env);

    let authorization = sign_migration_player(&env, &sk, &wallet, 1, &new_contract_hint, 1);

    let vitals = vitals(&env);
    let hashes = ipfs_hashes(&env);
    let level = ProgressLevel::Unverified;

    let result1 = client.try_redeem_migration_player(
        &wallet,
        &vitals,
        &hashes,
        &level,
        &1u64,
        &1_700_000_000u64,
        &1_700_000_000u64,
        &authorization,
    );
    assert!(result1.is_ok(), "First redemption should succeed");

    // Same authorization (same nonce) against a different player_id must be
    // rejected by the nonce replay guard before any state is written.
    let result2 = client.try_redeem_migration_player(
        &wallet,
        &vitals,
        &hashes,
        &level,
        &2u64,
        &1_700_000_000u64,
        &1_700_000_000u64,
        &authorization,
    );

    assert_eq!(
        result2,
        Err(Ok(ScoutChainError::InvalidInput)),
        "Replay with same nonce should be rejected"
    );
}
