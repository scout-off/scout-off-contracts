// fixtures/storage_compat_breaking/new_types.rs
// New types.rs snapshot — deliberately introduces a BREAKING change:
// the `registered_at` and `active` fields of Validator are reordered.
// check-storage-layout-compat.sh must exit 1 for this pair.

use soroban_sdk::{contracttype, Address, String};

#[contracttype]
#[derive(Clone, Debug)]
pub struct Validator {
    pub wallet: Address,
    pub credentials: String,
    // BREAKING: `active` moved before `registered_at` — positional
    // serialization means all existing stored Validator entries will
    // deserialise with the wrong field values after this upgrade.
    pub active: bool,
    pub registered_at: u64,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    Validator(Address),
    ValidatorVector,
}
