//! Issue #1399: milestone commit paths require an active registered player
//! when registration is wired; unwired registration remains permissive.

use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{
    RegPlayerProfile, RegPlayerVitals, VerificationContract, VerificationContractClient,
    VerificationError,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::Address as _,
    Address, Env, String, Vec,
};

#[contracttype]
enum RegStubKey {
    Owner,
    Missing,
    Deactivated,
}

#[contract]
struct RegStub;

#[contractimpl]
impl RegStub {
    pub fn initialize(env: Env, owner: Address) {
        env.storage().persistent().set(&RegStubKey::Owner, &owner);
    }

    pub fn set_missing(env: Env, player_id: u64) {
        env.storage()
            .persistent()
            .set(&RegStubKey::Missing, &player_id);
    }

    pub fn set_deactivated(env: Env, player_id: u64) {
        env.storage()
            .persistent()
            .set(&RegStubKey::Deactivated, &player_id);
    }

    pub fn get_player(env: Env, player_id: u64) -> RegPlayerProfile {
        let missing: Option<u64> = env.storage().persistent().get(&RegStubKey::Missing);
        if missing == Some(player_id) {
            panic!("player not registered");
        }
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

    pub fn is_player_deactivated(env: Env, player_id: u64) -> bool {
        let deactivated: Option<u64> = env.storage().persistent().get(&RegStubKey::Deactivated);
        deactivated == Some(player_id)
    }
}

const CID: &str = "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB";
const CID2: &str = "QmRhbYsqpiYgUY9KfNCcbfopHPbLnWSVKBpDNs37aZ3kVC";
const CID3: &str = "QmwsjoZwgfzgx6xPr3cXEKhzfLt5RQ87yMnWecTp1tf6p7";

fn base() -> (Env, VerificationContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(VerificationContract, ());
    let client = VerificationContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let validator = Address::generate(&env);
    client.register_validator(
        &validator,
        &String::from_str(&env, "UEFA-B-License-2026"),
        &String::from_str(&env, "Academy"),
        &Vec::new(&env),
    );
    (env, client, validator)
}

#[test]
fn unwired_registration_allows_approve() {
    let (env, client, validator) = base();
    let idx = client.approve_milestone(
        &validator,
        &42u64,
        &String::from_str(&env, "ok"),
        &String::from_str(&env, CID),
        &None,
    );
    assert_eq!(idx, 1);
}

#[test]
fn approve_rejects_unknown_and_deactivated_when_wired() {
    let (env, client, validator) = base();
    let owner = Address::generate(&env);
    let reg_id = env.register(RegStub, ());
    let reg = RegStubClient::new(&env, &reg_id);
    reg.initialize(&owner);
    client.set_registration_contract(&reg_id);

    reg.set_missing(&99u64);
    assert_eq!(
        client.try_approve_milestone(
            &validator,
            &99u64,
            &String::from_str(&env, "x"),
            &String::from_str(&env, CID),
            &None,
        ),
        Err(Ok(VerificationError::PlayerNotRegistered))
    );

    reg.set_deactivated(&7u64);
    assert_eq!(
        client.try_approve_milestone(
            &validator,
            &7u64,
            &String::from_str(&env, "x"),
            &String::from_str(&env, CID2),
            &None,
        ),
        Err(Ok(VerificationError::PlayerDeactivated))
    );

    // Active player still works.
    let idx = client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "ok"),
        &String::from_str(&env, CID3),
        &None,
    );
    assert_eq!(idx, 1);
}

#[test]
fn attest_rejects_deactivated_player_when_wired() {
    let (env, client, validator) = base();
    let owner = Address::generate(&env);
    let reg_id = env.register(RegStub, ());
    let reg = RegStubClient::new(&env, &reg_id);
    reg.initialize(&owner);
    client.set_registration_contract(&reg_id);
    reg.set_deactivated(&3u64);

    assert_eq!(
        client.try_attest_milestone(
            &validator,
            &3u64,
            &String::from_str(&env, "desc"),
            &String::from_str(&env, CID),
        ),
        Err(Ok(VerificationError::PlayerDeactivated))
    );
}
