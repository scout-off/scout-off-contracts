// fixtures/storage_compat_breaking/old_types.rs
// Baseline types.rs used by check-storage-layout-compat.sh breaking-change test.

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
