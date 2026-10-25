//! Test helpers shared by the contract crates (enabled by the `testutils`
//! feature).
//!
//! `Env::events().all()` returns the events of the most recent top-level
//! invocation as XDR, which is awkward to search by name. These helpers match
//! on the first topic, which every ScoutChain event sets to its event name.
extern crate std;

use soroban_sdk::testutils::Events as _;
use soroban_sdk::xdr::{ContractEvent, ContractEventBody, ScSymbol, ScVal};
use soroban_sdk::{Address, Env, TryFromVal, Val};

fn name_topic(name: &str) -> ScVal {
    ScVal::Symbol(ScSymbol(name.try_into().expect("event name too long")))
}

fn matches(event: &ContractEvent, wanted: &ScVal) -> bool {
    match &event.body {
        ContractEventBody::V0(v0) => v0.topics.first() == Some(wanted),
    }
}

/// Number of events named `name` in the most recent invocation, optionally
/// restricted to those emitted by `contract`.
pub fn count_events(env: &Env, name: &str, contract: Option<&Address>) -> usize {
    let wanted = name_topic(name);
    let all = env.events().all();
    let events = match contract {
        Some(addr) => all.filter_by_contract(addr),
        None => all,
    };
    events
        .events()
        .iter()
        .filter(|e| matches(e, &wanted))
        .count()
}

/// Whether the most recent invocation emitted an event named `name`.
pub fn has_event(env: &Env, name: &str) -> bool {
    count_events(env, name, None) > 0
}

/// Data payload of the last event named `name` in the most recent
/// invocation, if any.
pub fn last_event_data(env: &Env, name: &str) -> Option<Val> {
    let wanted = name_topic(name);
    env.events()
        .all()
        .events()
        .iter()
        .rev()
        .find(|e| matches(e, &wanted))
        .map(|e| match &e.body {
            ContractEventBody::V0(v0) => Val::try_from_val(env, &v0.data).unwrap(),
        })
}

/// Minimal peer contract for wiring tests: it only answers the wiring
/// handshake (`contract_kind` / `version`, see `verify_peer`) as the kind it
/// was deployed with. Use it wherever a test needs a peer address but not the
/// peer's behaviour.
#[soroban_sdk::contract]
pub struct PeerStub;

#[soroban_sdk::contracttype]
enum PeerStubKey {
    Kind,
}

#[soroban_sdk::contractimpl]
impl PeerStub {
    pub fn __constructor(env: Env, kind: soroban_sdk::Symbol) {
        env.storage().instance().set(&PeerStubKey::Kind, &kind);
    }

    pub fn contract_kind(env: Env) -> soroban_sdk::Symbol {
        env.storage().instance().get(&PeerStubKey::Kind).unwrap()
    }

    pub fn version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "2.0.0")
    }
}

/// Deploy a [`PeerStub`] that passes the wiring handshake as `kind`
/// (`"registration"`, `"verification"`, `"progress"` or `"scout_access"`).
pub fn peer_stub(env: &Env, kind: &str) -> Address {
    env.register(PeerStub, (soroban_sdk::Symbol::new(env, kind),))
}

/// Mirrors registration's `PlayerStatus` (same variant names, so it is
/// interchangeable on the wire).
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StubPlayerStatus {
    Active,
    Deactivated,
}

/// Registration peer that reports every player as `Active`. Wire it into
/// verification (`set_registration_contract`) in tests that commit milestones
/// but are not about registration itself: since #1622 verification fails
/// closed with `RegistrationNotConfigured` when no registration is wired.
#[soroban_sdk::contract]
pub struct ActiveRegistryStub;

#[soroban_sdk::contractimpl]
impl ActiveRegistryStub {
    pub fn contract_kind(env: Env) -> soroban_sdk::Symbol {
        soroban_sdk::Symbol::new(&env, "registration")
    }

    pub fn version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "2.0.0")
    }

    pub fn get_player_status(_env: Env, _player_id: u64) -> StubPlayerStatus {
        StubPlayerStatus::Active
    }
}

/// Deploy an [`ActiveRegistryStub`].
pub fn active_registry_stub(env: &Env) -> Address {
    env.register(ActiveRegistryStub, ())
}
