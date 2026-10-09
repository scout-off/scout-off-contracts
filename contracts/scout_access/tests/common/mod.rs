//! Shared test doubles for the scout_access integration tests.
//!
//! Since #1417 Pro/Elite subscriptions fail closed unless a registration
//! contract is wired, and since #1409 the progress contract refuses level
//! changes without one. `RegistrationStub` stands in for both roles: every
//! wallet is a verified, active scout, every level sync is accepted, and
//! player wallets can be mapped to ids for trial-offer ownership checks.
#![allow(dead_code)]

use scoutchain_progress::ProgressContractClient;
use scoutchain_scout_access::ScoutAccessContractClient;
use scoutchain_shared_types::ProgressLevel;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String};

#[contracttype]
#[derive(Clone, Debug)]
pub struct ScoutVerificationRecord {
    pub verified: bool,
    pub verified_by: Option<Address>,
    pub verified_at: Option<u64>,
    pub evidence_ref: Option<String>,
    pub method: Option<String>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ScoutProfile {
    pub scout_id: u64,
    pub wallet: Address,
    pub region: String,
    pub verified: bool,
    pub verification: ScoutVerificationRecord,
    pub registered_at: u64,
}

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum StubError {
    PlayerNotFound = 3,
}

#[contracttype]
enum StubKey {
    PlayerId(Address),
}

#[contract]
pub struct RegistrationStub;

#[contractimpl]
impl RegistrationStub {
    pub fn get_scout_by_wallet(env: Env, wallet: Address) -> ScoutProfile {
        ScoutProfile {
            scout_id: 1,
            wallet,
            region: String::from_str(&env, "NG"),
            verified: true,
            verification: ScoutVerificationRecord {
                verified: true,
                verified_by: None,
                verified_at: None,
                evidence_ref: None,
                method: None,
            },
            registered_at: 0,
        }
    }

    pub fn is_scout_deactivated(_env: Env, _scout_id: u64) -> bool {
        false
    }

    pub fn set_player_level(_env: Env, _player_id: u64, _level: ProgressLevel) {}

    pub fn map_player(env: Env, wallet: Address, player_id: u64) {
        env.storage()
            .persistent()
            .set(&StubKey::PlayerId(wallet), &player_id);
    }

    pub fn get_player_id_by_wallet(env: Env, wallet: Address) -> Result<u64, StubError> {
        env.storage()
            .persistent()
            .get(&StubKey::PlayerId(wallet))
            .ok_or(StubError::PlayerNotFound)
    }
}

/// Deploy a `RegistrationStub` and wire it into whichever of `scout_access`
/// and `progress` are given. Returns its client.
pub fn wire_registration(
    env: &Env,
    scout_access: Option<&ScoutAccessContractClient>,
    progress: Option<&ProgressContractClient>,
) -> RegistrationStubClient<'static> {
    let id = env.register(RegistrationStub, ());
    if let Some(sa) = scout_access {
        sa.set_registration_contract(&id);
    }
    if let Some(p) = progress {
        p.set_registration_contract(&id);
    }
    RegistrationStubClient::new(env, &id)
}

/// Progress stand-in reporting every player as Unverified. Pro-tier contacts
/// fail closed when no progress contract is wired (#1357); use this where a
/// test needs Pro contacts but not real level tracking.
#[contract]
pub struct ProgressLevelStub;

#[contractimpl]
impl ProgressLevelStub {
    pub fn get_level(_env: Env, _player_id: u64) -> ProgressLevel {
        ProgressLevel::Unverified
    }
}

/// Wire a `ProgressLevelStub` as `scout_access`'s progress contract.
pub fn wire_progress_level_stub(env: &Env, scout_access: &ScoutAccessContractClient) {
    scout_access.set_progress_contract(&env.register(ProgressLevelStub, ()));
}
