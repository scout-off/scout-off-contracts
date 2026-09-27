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
    /// Whether the contract has completed its one-time initialization.
    pub initialized: bool,
    /// Whether state-changing operations are currently paused.
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

/// Adapter trait for contract-specific error enums used by the shared
/// admin-authorization helpers.
///
/// Implementing this trait lets each contract's error enum plug into the
/// common `require_admin`, `propose_admin`, and `accept_admin` helper pattern
/// while still returning that contract's own error type.
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
pub fn require_admin<K, E>(env: &Env, admin_key: &K, admin_bump_ledgers: u32) -> Result<Address, E>
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

/// Lightweight syntactic validation for IPFS CIDs used as evidence and media references.
///
/// Performs a format sanity check on the supplied CID string. This is **not** a
/// full CID decoder — it does not parse the multibase prefix, multicodec, or
/// multihash the way a real CID library would. It only rejects obviously wrong
/// input (wrong prefix, wrong length, or bytes outside the expected alphabet).
/// A CID that passes this check but is still malformed will fail to resolve
/// against the downstream IPFS/Arweave gateway, which acts as the real source
/// of truth for CID validity.
///
/// # Rules
///
/// - **CIDv0**: starts with `"Qm"`, exactly 46 characters, base58btc charset
///   (no `0`, `O`, `I`, or `l` characters).
/// - **CIDv1 (base32)**: starts with `"bafy"`, 59–128 characters, RFC 4648
///   lowercase base32 alphabet (`a`–`z`, `2`–`7`).
///
/// # Errors
///
/// Returns `Err(&'static str)` with a human-readable message describing the
/// validation failure. These messages are intended for tests and debugging;
/// callers should map them to the appropriate contract error variant (e.g.
/// `InvalidInput`) rather than surfacing the raw string to end users.
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
        // Base58btc charset only (alphanumeric, excluding 0, O, I, l) — this
        // rejects whitespace, control characters, and any other byte outside
        // the alphabet, not just the four excluded look-alike characters.
        for i in 0..hash_len {
            match bytes.get(i) {
                Some(b) if is_base58btc_char(b) => {}
                _ => {
                    return Err("invalid cid: CIDv0 contains invalid base58btc character");
                }
            }
        }
        Ok(())
    } else if starts_with_bafy {
        // CIDv1 (base32): 59–128 chars, RFC4648 lowercase base32 charset
        // (a–z, 2–7). This is a lightweight format sanity check, not a full
        // CID decoder — it does not parse the multibase prefix, multicodec,
        // or multihash the way a real CID library would. Any CID that
        // passes this check but is still malformed will simply fail to
        // resolve against the downstream IPFS/Arweave gateway, which acts
        // as the real source of truth for CID validity. This function only
        // needs to catch obviously wrong input (wrong prefix, wrong length,
        // or bytes outside the expected alphabet — e.g. whitespace or
        // control characters), not guarantee byte-for-byte correctness.
        if !(59..=128).contains(&hash_len) {
            return Err("invalid cid: CIDv1 must be 59–128 characters");
        }
        for i in 0..hash_len {
            match bytes.get(i) {
                Some(b) if is_base32_char(b) => {}
                _ => {
                    return Err("invalid cid: CIDv1 contains invalid base32 character");
                }
            }
        }
        Ok(())
    } else {
        Err("invalid cid: must start with 'Qm' (CIDv0) or 'bafy' (CIDv1)")
    }
}

/// Base58btc alphabet: digits 1–9, uppercase A–Z except I/O, lowercase a–z
/// except l.
fn is_base58btc_char(b: u8) -> bool {
    matches!(b,
        b'1'..=b'9'
        | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z'
        | b'a'..=b'k' | b'm'..=b'z'
    )
}

/// RFC4648 lowercase base32 alphabet: a–z and 2–7.
fn is_base32_char(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'2'..=b'7')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(env: &Env, v: &str) -> String {
        String::from_str(env, v)
    }

    #[test]
    fn test_validate_cid_v0_accepts_valid() {
        let env = Env::default();
        let cid = s(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");
        assert!(validate_cid(&cid).is_ok());
    }

    #[test]
    fn test_validate_cid_v0_rejects_space_in_body() {
        let env = Env::default();
        // Still 46 chars and "Qm"-prefixed, but the last byte is a space
        // instead of a base58btc character.
        let cid = s(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygpq ");
        assert!(validate_cid(&cid).is_err());
    }

    #[test]
    fn test_validate_cid_v0_rejects_newline_in_body() {
        let env = Env::default();
        let cid = s(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygpq\n");
        assert!(validate_cid(&cid).is_err());
    }

    #[test]
    fn test_validate_cid_v0_rejects_null_byte_in_body() {
        let env = Env::default();
        let cid = s(&env, "QmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4Ygpq\0");
        assert!(validate_cid(&cid).is_err());
    }

    #[test]
    fn test_validate_cid_v1_accepts_valid() {
        let env = Env::default();
        let cid = s(
            &env,
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
        );
        assert!(validate_cid(&cid).is_ok());
    }

    #[test]
    fn test_validate_cid_v1_rejects_whitespace_in_body() {
        let env = Env::default();
        let cid = s(
            &env,
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzd ",
        );
        assert!(validate_cid(&cid).is_err());
    }

    #[test]
    fn test_validate_cid_v1_rejects_uppercase_in_body() {
        let env = Env::default();
        // Uppercase letters are outside the lowercase base32 alphabet.
        let cid = s(
            &env,
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzDI",
        );
        assert!(validate_cid(&cid).is_err());
    }

    #[test]
    fn test_validate_cid_rejects_bad_prefix() {
        let env = Env::default();
        let cid = s(&env, "XmPK1s3pNYLi9ERiq3BDxKa4XosgWwFRQUydHUtz4YgpqB");
        assert!(validate_cid(&cid).is_err());
    }
}
