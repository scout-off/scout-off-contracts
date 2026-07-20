#![no_std]
use soroban_sdk::{contracttype, Address, Env, IntoVal, String};

/// Four-tier progress level for a player profile
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProgressLevel {
    /// Level 0 — profile created, no verification yet
    Unverified,
    /// Level 1 — identity confirmed by academy or KYC
    VerifiedIdentity,
    /// Level 2 — performance milestones verified by approved third party
    PerformanceMilestones,
    /// Level 3 — scout feedback or trial offer logged
    EliteTier,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractHealth {
    pub initialized: bool,
    pub paused: bool,
}

impl ProgressLevel {
    /// Returns the next valid level, or None if already at the top.
    pub fn next(&self) -> Option<ProgressLevel> {
        match self {
            ProgressLevel::Unverified => Some(ProgressLevel::VerifiedIdentity),
            ProgressLevel::VerifiedIdentity => Some(ProgressLevel::PerformanceMilestones),
            ProgressLevel::PerformanceMilestones => Some(ProgressLevel::EliteTier),
            ProgressLevel::EliteTier => None,
        }
    }
}

/// Trait for contract-specific error enums that can produce admin-related
/// errors. Each contract implements this trait on its own error type so the
/// shared `require_admin` helper can return the correct per-contract error.
pub trait AdminError {
    /// Return the "contract not initialized" error variant for this contract.
    fn not_initialized() -> Self;
}

/// Shared admin-authorization helper.
///
/// Reads the stored admin `Address` from persistent storage using `admin_key`,
/// calls [`Address::require_auth`] on it, extends the key's TTL by
/// `admin_bump_ledgers`, and returns the admin address.
///
/// # Generic parameters
/// - `K` — the storage key type (each contract defines its own `DataKey` enum;
///   pass `&DataKey::Admin`).
/// - `E` — the contract-specific error type, which must implement
///   [`AdminError`].
///
/// # Errors
/// Returns `E::not_initialized()` when the admin key is absent from
/// persistent storage.
///
/// # Usage
///
/// ```ignore
/// use scoutchain_shared_types::require_admin;
///
/// // Inside a contract function returning Result<(), MyError>:
/// let admin = require_admin(&env, &DataKey::Admin, ADMIN_BUMP_LEDGERS)?;
/// ```
pub fn require_admin<K, E>(
    env: &Env,
    admin_key: &K,
    admin_bump_ledgers: u32,
) -> Result<Address, E>
where
    K: IntoVal<Env, soroban_sdk::Val>,
    E: AdminError,
{
    let admin: Address = env
        .storage()
        .persistent()
        .get(admin_key)
        .ok_or_else(|| E::not_initialized())?;
    admin.require_auth();
    env.storage()
        .persistent()
        .extend_ttl(admin_key, admin_bump_ledgers, admin_bump_ledgers);
    Ok(admin)
}

/// Validate that a string is a plausible IPFS/Arweave CID.
///
/// Rules:
/// - CIDv0: starts with "Qm", exactly 46 characters, base58btc charset
///   (no 0, O, I, l characters).
/// - CIDv1 (base32): starts with "bafy", 59–128 characters.
pub fn validate_cid(hash: &String) -> Result<(), &'static str> {
    let hash_len = hash.len();
    let bytes = hash.to_bytes();

    let starts_with_qm = bytes.get(0) == Some(b'Q') && bytes.get(1) == Some(b'm');
    let starts_with_bafy = hash_len >= 4
        && bytes.get(0) == Some(b'b')
        && bytes.get(1) == Some(b'a')
        && bytes.get(2) == Some(b'f')
        && bytes.get(3) == Some(b'y');

    if starts_with_qm {
        // CIDv0: exactly 46 chars
        if hash_len != 46 {
            return Err("invalid cid: CIDv0 must be exactly 46 characters");
        }
        // Base58btc: no 0, O, I, l
        for i in 0..hash_len {
            match bytes.get(i) {
                Some(b'0') | Some(b'O') | Some(b'I') | Some(b'l') => {
                    return Err("invalid cid: CIDv0 contains invalid base58btc character");
                }
                _ => {}
            }
        }
        Ok(())
    } else if starts_with_bafy {
        // CIDv1 (base32): 59–128 chars
        if !(59..=128).contains(&hash_len) {
            return Err("invalid cid: CIDv1 must be 59–128 characters");
        }
        Ok(())
    } else {
        Err("invalid cid: must start with 'Qm' (CIDv0) or 'bafy' (CIDv1)")
    }
}
