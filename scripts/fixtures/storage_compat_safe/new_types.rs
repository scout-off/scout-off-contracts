// fixtures/storage_compat_safe/new_types.rs
// New types.rs snapshot — only SAFE changes: new field appended to struct,
// new variant appended to DataKey enum.
// check-storage-layout-compat.sh must exit 0 for this pair.

use soroban_sdk::{contracttype, Address, String};

#[contracttype]
#[derive(Clone, Debug)]
pub struct Validator {
    pub wallet: Address,
    pub credentials: String,
    pub registered_at: u64,
    pub active: bool,
    // SAFE: new field appended at end — existing stored values are still
    // deserializable; the new field is absent in old entries (handled by
    // callers via Option or default).
    pub region: String,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    Validator(Address),
    ValidatorVector,
    // SAFE: new variant appended — existing discriminant values are unchanged.
    MinRegionQuorum,
}
