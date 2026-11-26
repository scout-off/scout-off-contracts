//! Erasure interlock tests (issue #1807): `purge_player_data` on progress,
//! verification and scout_access must fail unless erasure is pending.

use scoutchain_progress::{ProgressContract, ProgressContractClient};
use scoutchain_registration::{PlayerVitals, RegistrationContract, RegistrationContractClient};
use scoutchain_scout_access::{FeeConfig, ScoutAccessContract, ScoutAccessContractClient};
use scoutchain_verification::{VerificationContract, VerificationContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

struct H {
    env: Env,
    reg: RegistrationContractClient<'static>,
    prog: ProgressContractClient<'static>,
    ver: VerificationContractClient<'static>,
    sa: ScoutAccessContractClient<'static>,
}

fn build(wire: bool) -> H {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let reg_id = env.register(RegistrationContract, (admin.clone(),));
    let reg = RegistrationContractClient::new(&env, &reg_id);
    let ver_id = env.register(VerificationContract, (admin.clone(),));
    let ver = VerificationContractClient::new(&env, &ver_id);
    let prog_id = env.register(ProgressContract, (admin.clone(),));
    let prog = ProgressContractClient::new(&env, &prog_id);
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let fees = FeeConfig {
        contact_fee_stroops: 100_000,
        basic_sub_stroops: 1_000_000,
        pro_sub_stroops: 3_000_000,
        elite_sub_stroops: 7_000_000,
        sub_duration_secs: 30 * 24 * 60 * 60,
        pro_contact_limit: 10,
        trial_offer_escrow_stroops: 500_000,
        trial_offer_expiry_secs: 3_600,
    };
    let sa_id = env.register(ScoutAccessContract, (admin.clone(), xlm, fees));
    let sa = ScoutAccessContractClient::new(&env, &sa_id);
    if wire {
        prog.set_registration_contract(&reg_id);
        ver.set_registration_contract(&reg_id);
        sa.set_registration_contract(&reg_id);
    }
    H { env, reg, prog, ver, sa }
}

fn register(h: &H) -> u64 {
    let env = &h.env;
    let vitals = PlayerVitals {
        age: 22,
        position: String::from_str(env, "CM"),
        region: String::from_str(env, "EU"),
        nationality: String::from_str(env, "BR"),
    };
    let mut hashes = Vec::new(env);
    hashes.push_back(String::from_str(
        env,
        "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB",
    ));
    h.reg.register_player(&Address::generate(env), &vitals, &hashes)
}

#[test]
fn purge_without_begin_erasure_is_rejected_everywhere() {
    let h = build(true);
    let id = register(&h);
    assert!(h.prog.try_purge_player_data(&id, &0u32).is_err());
    assert!(h.ver.try_purge_player_data(&id, &0u32).is_err());
    assert!(h.sa.try_purge_player_data(&id, &0u32).is_err());
    assert!(!h.reg.is_erasure_pending(&id));
}

#[test]
fn purge_after_begin_erasure_proceeds() {
    let h = build(true);
    let id = register(&h);
    h.reg.begin_erasure(&id);
    assert!(h.reg.is_erasure_pending(&id));
    assert!(h.prog.try_purge_player_data(&id, &0u32).is_ok());
    assert!(h.ver.try_purge_player_data(&id, &0u32).is_ok());
    assert!(h.sa.try_purge_player_data(&id, &0u32).is_ok());
    // Deregistering completes erasure and clears the marker.
    h.reg.deregister_player(&id);
    assert!(!h.reg.is_erasure_pending(&id));
    // Player gone: purges remain re-runnable.
    assert!(h.prog.try_purge_player_data(&id, &0u32).is_ok());
}

#[test]
fn purge_fails_closed_when_registration_unwired() {
    let h = build(false);
    assert!(h.prog.try_purge_player_data(&1u64, &0u32).is_err());
    assert!(h.ver.try_purge_player_data(&1u64, &0u32).is_err());
    assert!(h.sa.try_purge_player_data(&1u64, &0u32).is_err());
}
