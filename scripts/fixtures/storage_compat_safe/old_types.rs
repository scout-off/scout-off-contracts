// fixtures/storage_compat_safe/old_types.rs
// Baseline types.rs snapshot used by check-storage-layout-compat.sh tests.
// This represents the BEFORE state (no new field yet).

use soroban_sdk::{contracttype, Address, String};

#[contracttype]
#[derive(Clone, Debug)]
pub struct Validator {
    pub wallet: Address,
    pub credentials: String,
    pub registered_at: u64,
    pub active: bool,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    Validator(Address),
    ValidatorVector,
}
