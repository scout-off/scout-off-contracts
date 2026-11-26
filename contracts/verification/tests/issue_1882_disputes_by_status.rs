//! Issue #1882: tests for `get_player_disputes_by_status`.

use scoutchain_shared_types::ProgressLevel;
use scoutchain_verification::{
    RegPlayerProfile, RegPlayerVitals, VerificationContract, VerificationContractClient,
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

fn approve(
    env: &Env,
    client: &VerificationContractClient<'static>,
    validator: &Address,
    cid: usize,
) -> u32 {
    client.approve_milestone(
        validator,
        &1u64,
        &String::from_str(env, "goal"),
        &String::from_str(env, CIDS[cid]),
        &None,
    )
}

fn dispute(env: &Env, client: &VerificationContractClient<'static>, player: &Address, idx: u32) {
    client.dispute_milestone(player, &1u64, &idx, &String::from_str(env, "wrong"), &0u32);
}

#[test]
fn no_disputes_returns_empty_for_both_filters() {
    let (_env, client, _player, _validator) = setup();
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &false, &0u32, &10u32)
            .len(),
        0
    );
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &true, &0u32, &10u32)
            .len(),
        0
    );
}

#[test]
fn open_and_resolved_filters_each_return_one() {
    let (env, client, player, validator) = setup();
    let a = approve(&env, &client, &validator, 0);
    let b = approve(&env, &client, &validator, 1);
    dispute(&env, &client, &player, a);
    dispute(&env, &client, &player, b);
    client.resolve_dispute(&1u64, &a, &true);

    let open = client.get_player_disputes_by_status(&1u64, &false, &0u32, &10u32);
    assert_eq!(open.len(), 1);
    assert_eq!(open.get(0).unwrap().milestone_index, b);
    assert!(!open.get(0).unwrap().resolved);

    let resolved = client.get_player_disputes_by_status(&1u64, &true, &0u32, &10u32);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved.get(0).unwrap().milestone_index, a);
    assert!(resolved.get(0).unwrap().resolved);
    assert!(resolved.get(0).unwrap().upheld);
}

#[test]
fn redispute_reports_only_latest_round_per_milestone() {
    let (env, client, player, validator) = setup();
    let idx = approve(&env, &client, &validator, 0);
    dispute(&env, &client, &player, idx);
    client.resolve_dispute(&1u64, &idx, &false);

    // Round 0 is resolved; nothing open yet.
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &false, &0u32, &10u32)
            .len(),
        0
    );
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &true, &0u32, &10u32)
            .len(),
        1
    );

    env.ledger().with_mut(|l| l.timestamp += COOLDOWN + 1);
    dispute(&env, &client, &player, idx);

    // Round 1 is open: the milestone appears once, under "open" only.
    let open = client.get_player_disputes_by_status(&1u64, &false, &0u32, &10u32);
    assert_eq!(open.len(), 1);
    assert_eq!(open.get(0).unwrap().round, 1);
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &true, &0u32, &10u32)
            .len(),
        0
    );
}

#[test]
fn pagination_and_limit_respected() {
    let (env, client, player, validator) = setup();
    for cid in 0..4 {
        let idx = approve(&env, &client, &validator, cid);
        dispute(&env, &client, &player, idx);
    }
    let all = client.get_player_disputes_by_status(&1u64, &false, &0u32, &10u32);
    assert_eq!(all.len(), 4);

    let page1 = client.get_player_disputes_by_status(&1u64, &false, &0u32, &2u32);
    assert_eq!(page1.len(), 2);
    let page2 = client.get_player_disputes_by_status(&1u64, &false, &2u32, &2u32);
    assert_eq!(page2.len(), 2);
    assert_eq!(
        page1.get(0).unwrap().milestone_index,
        all.get(0).unwrap().milestone_index
    );
    assert_eq!(
        page2.get(0).unwrap().milestone_index,
        all.get(2).unwrap().milestone_index
    );

    // Offset past the end yields nothing; limit 0 is clamped to 1.
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &false, &4u32, &2u32)
            .len(),
        0
    );
    assert_eq!(
        client
            .get_player_disputes_by_status(&1u64, &false, &0u32, &0u32)
            .len(),
        1
    );
}

#[test]
fn offset_applies_after_status_filter() {
    let (env, client, player, validator) = setup();
    let a = approve(&env, &client, &validator, 0);
    let b = approve(&env, &client, &validator, 1);
    let c = approve(&env, &client, &validator, 2);
    for i in [a, b, c] {
        dispute(&env, &client, &player, i);
    }
    client.resolve_dispute(&1u64, &a, &false);
    // Open = [b, c]; offset 1 must skip only b, not the resolved a.
    let open = client.get_player_disputes_by_status(&1u64, &false, &1u32, &10u32);
    assert_eq!(open.len(), 1);
    assert_eq!(open.get(0).unwrap().milestone_index, c);
}
