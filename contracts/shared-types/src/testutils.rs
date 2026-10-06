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
