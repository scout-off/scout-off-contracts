//! Issue #1396: dispute rounds (re-dispute after cooldown) + open-dispute cap.

use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{
    RegPlayerProfile, RegPlayerVitals, VerificationContract, VerificationContractClient,
    VerificationError,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

#[contracttype]
enum RegStubKey {
    Owner,
}

#[contract]
struct RegStub;

#[contractimpl]
impl RegStub {
    /// The owner wallet owns player 1, the only player these tests dispute.
    pub fn get_player_id_by_wallet(env: Env, wallet: Address) -> u64 {
        let owner: Address = env.storage().persistent().get(&RegStubKey::Owner).unwrap();
        if wallet == owner {
            1
        } else {
            0
        }
    }

    /// Every player counts as active for verification's #1622 check.
    pub fn get_player_status(
        _env: Env,
        _player_id: u64,
    ) -> scoutchain_verification::RegPlayerStatus {
        scoutchain_verification::RegPlayerStatus::Active
    }

    /// Wiring handshake (#1610): report the kind of contract this stub stands in for.
    pub fn contract_kind(env: Env) -> soroban_sdk::Symbol {
        soroban_sdk::Symbol::new(&env, "registration")
    }

    pub fn version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "2.0.0")
    }

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
            ipfs_hashes: Vec::new(&env),
            level: ProgressLevel::Unverified,
            registered_at: 0,
            updated_at: 0,
        }
    }

    pub fn is_player_deactivated(_env: Env, _player_id: u64) -> bool {
        false
    }
}

const COOLDOWN: u64 = 604_800;
const CIDS: [&str; 6] = [
    "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB",
    "QmRhbYsqpiYgUY9KfNCcbfopHPbLnWSVKBpDNs37aZ3kVC",
    "QmwsjoZwgfzgx6xPr3cXEKhzfLt5RQ87yMnWecTp1tf6p7",
    "QmgzsER5ykyxoTsVUSePRkKXqkEzsRVLpUv511dp4c3vAs",
    "QmABCDEFGHJKLMNPQRSTUVWXYZ123456789abcdefghijk",
    "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
];

fn setup() -> (Env, VerificationContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let player = Address::generate(&env);
    let id = env.register(VerificationContract, (admin.clone(),));
    let client = VerificationContractClient::new(&env, &id);
    let reg_id = env.register(RegStub, ());
    RegStubClient::new(&env, &reg_id).initialize(&player);
    client.set_registration_contract(&reg_id);
    let validator = Address::generate(&env);
    client.register_validator(
        &validator,
        &String::from_str(&env, "UEFA-B-License-2026"),
        &String::from_str(&env, "Academy"),
        &Vec::new(&env),
    );
    (env, client, player, validator)
}

#[test]
fn redispute_blocked_before_cooldown_allowed_after() {
    let (env, client, player_wallet, validator) = setup();
    let idx = client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "goal"),
        &String::from_str(&env, CIDS[0]),
        &None,
    );

    client.dispute_milestone(
        &player_wallet,
        &1u64,
        &idx,
        &String::from_str(&env, "wrong"),
        &0u32,
    );
    assert_eq!(
        client.try_dispute_milestone(
            &player_wallet,
            &1u64,
            &idx,
            &String::from_str(&env, "again"),
            &0u32,
        ),
        Err(Ok(VerificationError::DisputeAlreadyOpen))
    );

    client.resolve_dispute(&1u64, &idx, &false);

    assert_eq!(
        client.try_dispute_milestone(
            &player_wallet,
            &1u64,
            &idx,
            &String::from_str(&env, "new evidence"),
            &0u32,
        ),
        Err(Ok(VerificationError::DisputeCooldown))
    );

    env.ledger().with_mut(|l| {
        l.timestamp += COOLDOWN + 1;
    });
    client.dispute_milestone(
        &player_wallet,
        &1u64,
        &idx,
        &String::from_str(&env, "new evidence"),
        &0u32,
    );
    let d = client.get_dispute(&1u64, &idx);
    assert_eq!(d.round, 1);
    assert!(!d.resolved);
}

#[test]
fn open_dispute_cap_enforced() {
    let (env, client, player_wallet, _validator) = setup();
    // Use distinct validators so we do not hit the per-(validator, player)
    // milestone cap of 5 while opening MAX_OPEN_DISPUTES_PER_PLAYER disputes.
    for cid in CIDS.iter().take(5) {
        let validator = Address::generate(&env);
        client.register_validator(
            &validator,
            &String::from_str(&env, "UEFA-B-License-2026"),
            &String::from_str(&env, "Academy"),
            &Vec::new(&env),
        );
        let idx = client.approve_milestone(
            &validator,
            &1u64,
            &String::from_str(&env, "Milestone m"),
            &String::from_str(&env, cid),
            &None,
        );
        client.dispute_milestone(
            &player_wallet,
            &1u64,
            &idx,
            &String::from_str(&env, "x"),
            &0u32,
        );
    }

    let validator = Address::generate(&env);
    client.register_validator(
        &validator,
        &String::from_str(&env, "UEFA-B-License-2026"),
        &String::from_str(&env, "Academy"),
        &Vec::new(&env),
    );
    let idx6 = client.approve_milestone(
        &validator,
        &1u64,
        &String::from_str(&env, "Milestone m"),
        &String::from_str(&env, CIDS[5]),
        &None,
    );
    assert_eq!(
        client.try_dispute_milestone(
            &player_wallet,
            &1u64,
            &idx6,
            &String::from_str(&env, "spam"),
            &0u32,
        ),
        Err(Ok(VerificationError::TooManyOpenDisputes))
    );
}
