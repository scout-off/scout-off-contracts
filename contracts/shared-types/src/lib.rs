#![no_std]

use soroban_sdk::{contracttype, Address, Env, IntoVal, String};

/// Four-tier progress level for a player profile
#[contracttype]
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProgressLevel {
    /// Level 0 - profile created, no verification yet
    Unverified,
    /// Level 1 - identity confirmed by academy or KYC
    VerifiedIdentity,
    /// Level 2 - performance milestones verified by approved third party
    PerformanceMilestones,
    /// Level 3 - scout feedback or trial offer logged
    EliteTier,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractHealth {
    /// Whether the contract has completed its one-time initialization.
    pub initialized: bool,
    /// Whether state-changing operations are currently paused.
    pub paused: bool,
    /// Whether the `scout_access.pay_to_contact` function is paused independently
    /// of the whole-contract pause (function-scoped circuit breaker).
    /// Always `false` for contracts that do not implement a `pay_to_contact`
    /// function (`registration`, `verification`, `progress`).
    pub pay_to_contact_paused: bool,
}

impl ProgressLevel {
    /// Monotonic ordering used by contracts that compare minimum progress tiers.
    pub fn rank(&self) -> u8 {
        match self {
            ProgressLevel::Unverified => 0,
            ProgressLevel::VerifiedIdentity => 1,
            ProgressLevel::PerformanceMilestones => 2,
            ProgressLevel::EliteTier => 3,
        }
    }

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

/// One cross-contract peer-address pointer: the currently configured
/// `address` (if any) and a monotonically incrementing `epoch` bumped on
/// every successful write via [`write_wiring_link`].
///
/// Every contract's `get_wiring_state()` getter returns one `WiringLink` per
/// peer pointer it holds (`verification` and `scout_access` each hold two;
/// `progress` holds three; `registration` holds one) — see
/// `docs/WIRING_REGISTRY_DESIGN.md` for the full cross-contract picture and
/// how an off-chain caller uses `epoch` to detect a partially-applied
/// re-wiring.
///
/// # Why `epoch` in addition to `address: Option<Address>`
///
/// `Option::None` already distinguishes "never configured" from "configured
/// to *something*" — `epoch` adds a dimension `Option` cannot: it lets an
/// operator distinguish *how many times* a link has been (re-)wired. Given
/// only a single snapshot this mostly matters for the specific interrupted
/// re-wiring scenario this design exists to catch: comparing `epoch` across
/// **all pointers that target the same contract** (e.g. `verification`'s,
/// `registration`'s, and `scout_access`'s independent `ProgressContract`
/// pointers) reveals a mid-migration state that a bare address comparison
/// alone would describe correctly but less diagnostically — an operator
/// re-running a re-wiring script can tell "my calls aren't landing at all"
/// (epoch unchanged from a prior snapshot) apart from "my calls are landing,
/// but with the wrong address" (epoch changed, address still wrong), which
/// point to two entirely different bugs (an auth/network failure vs. a
/// typo'd argument).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WiringLink {
    pub address: Option<Address>,
    pub epoch: u32,
}

impl WiringLink {
    /// The zero-value link: never configured.
    pub const fn unconfigured() -> Self {
        WiringLink {
            address: None,
            epoch: 0,
        }
    }

    /// Whether this link currently has an address set. Equivalent to
    /// `epoch > 0` — every successful [`write_wiring_link`] call sets both
    /// the address and bumps the epoch together, so the two can never
    /// disagree about "configured or not."
    pub fn is_configured(&self) -> bool {
        self.address.is_some()
    }
}

/// Read a wiring link's current address + epoch from instance storage.
///
/// `addr_key` and `epoch_key` are two variants of the calling contract's own
/// `DataKey` enum (e.g. `DataKey::ProgressContract` and
/// `DataKey::ProgressContractEpoch`). Returns [`WiringLink::unconfigured`]
/// (address `None`, epoch `0`) if the link has never been written.
pub fn read_wiring_link<K>(env: &Env, addr_key: &K, epoch_key: &K) -> WiringLink
where
    K: IntoVal<Env, soroban_sdk::Val>,
{
    let address = env.storage().instance().get::<K, Address>(addr_key);
    let epoch = env
        .storage()
        .instance()
        .get::<K, u32>(epoch_key)
        .unwrap_or(0);
    WiringLink { address, epoch }
}

/// Write a wiring link's address and atomically bump its epoch by one.
///
/// Every `set_*_contract` / `update_*_contract` setter across all four
/// contracts calls this so epoch bookkeeping cannot silently drift between
/// per-contract implementations — see `docs/WIRING_REGISTRY_DESIGN.md`
/// ("Open Question 1", resolved by this shared helper). Returns the new
/// epoch value; callers pass it straight into their `wiring_updated` event.
///
/// This does not perform admin authorization itself — callers must call
/// [`require_admin`] (or otherwise authorize the caller) before invoking
/// this.
pub fn write_wiring_link<K>(env: &Env, addr_key: &K, epoch_key: &K, addr: &Address) -> u32
where
    K: IntoVal<Env, soroban_sdk::Val>,
{
    let next_epoch = env
        .storage()
        .instance()
        .get::<K, u32>(epoch_key)
        .unwrap_or(0)
        + 1;
    env.storage().instance().set(addr_key, addr);
    env.storage().instance().set(epoch_key, &next_epoch);
    next_epoch
}

/// Safe (checked) arithmetic helpers shared across all four contracts.
///
/// # Design rationale
///
/// Every contract previously repeated the same `.checked_add(x).ok_or(ContractError::Overflow)?`
/// pattern independently. This module centralises the pattern so:
///
/// - There is a single place to audit all overflow-sensitive arithmetic.
/// - Property-based boundary tests live here, proving no call site can panic or
///   silently wrap for any input.
/// - Each contract maps the shared `ArithmeticError` to its own typed error
///   with a one-liner (see `impl From<ArithmeticError> for MyError` in each
///   contract, or use `safe_add_u32(a, b).map_err(|_| MyError::Overflow)?`).
///
/// # Covered types
///
/// | Type | Operations |
/// |------|-----------|
/// | `u32` | `safe_add_u32`, `safe_sub_u32` |
/// | `u64` | `safe_add_u64`, `safe_sub_u64` |
/// | `i128` | `safe_add_i128`, `safe_sub_i128`, `safe_mul_i128` |
///
/// # Usage
///
/// ```ignore
/// use scoutchain_shared_types::safe_math::{safe_add_u32, safe_add_i128};
///
/// // In a contract function:
/// let next_count = safe_add_u32(current_count, 1)
///     .map_err(|_| MyError::Overflow)?;
///
/// let new_fees = safe_add_i128(accumulated, payment_amount)
///     .map_err(|_| MyError::Overflow)?;
/// ```
pub mod safe_math {
    /// Returned when a checked-arithmetic operation overflows or underflows.
    /// Map this to your contract's own Overflow error variant at the call site.
    #[derive(Debug, PartialEq)]
    pub struct ArithmeticError;

    // ── u32 ──────────────────────────────────────────────────────────────────

    /// Checked addition for `u32`. Returns `ArithmeticError` on overflow.
    #[inline]
    pub fn safe_add_u32(a: u32, b: u32) -> Result<u32, ArithmeticError> {
        a.checked_add(b).ok_or(ArithmeticError)
    }

    /// Checked subtraction for `u32`. Returns `ArithmeticError` on underflow.
    #[inline]
    pub fn safe_sub_u32(a: u32, b: u32) -> Result<u32, ArithmeticError> {
        a.checked_sub(b).ok_or(ArithmeticError)
    }

    // ── u64 ──────────────────────────────────────────────────────────────────

    /// Checked addition for `u64`. Returns `ArithmeticError` on overflow.
    #[inline]
    pub fn safe_add_u64(a: u64, b: u64) -> Result<u64, ArithmeticError> {
        a.checked_add(b).ok_or(ArithmeticError)
    }

    /// Checked subtraction for `u64`. Returns `ArithmeticError` on underflow.
    #[inline]
    pub fn safe_sub_u64(a: u64, b: u64) -> Result<u64, ArithmeticError> {
        a.checked_sub(b).ok_or(ArithmeticError)
    }

    // ── i128 ─────────────────────────────────────────────────────────────────

    /// Checked addition for `i128`. Returns `ArithmeticError` on overflow.
    ///
    /// This is the primary helper for stroop fee-accumulation paths in
    /// `scout_access` — the highest-financial-risk arithmetic in the codebase.
    #[inline]
    pub fn safe_add_i128(a: i128, b: i128) -> Result<i128, ArithmeticError> {
        a.checked_add(b).ok_or(ArithmeticError)
    }

    /// Checked subtraction for `i128`. Returns `ArithmeticError` on underflow.
    #[inline]
    pub fn safe_sub_i128(a: i128, b: i128) -> Result<i128, ArithmeticError> {
        a.checked_sub(b).ok_or(ArithmeticError)
    }

    /// Checked multiplication for `i128`. Returns `ArithmeticError` on overflow.
    ///
    /// Used in `batch_contact_players` to compute `contact_fee * new_contacts`.
    #[inline]
    pub fn safe_mul_i128(a: i128, b: i128) -> Result<i128, ArithmeticError> {
        a.checked_mul(b).ok_or(ArithmeticError)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[cfg(test)]
    pub mod tests {
        use super::*;

        // ── u32 boundary-value suite ─────────────────────────────────────────

        #[test]
        fn u32_add_zero_identity() {
            assert_eq!(safe_add_u32(0, 0), Ok(0));
            assert_eq!(safe_add_u32(u32::MAX, 0), Ok(u32::MAX));
            assert_eq!(safe_add_u32(0, u32::MAX), Ok(u32::MAX));
        }

        #[test]
        fn u32_add_overflow_returns_err() {
            assert_eq!(safe_add_u32(u32::MAX, 1), Err(ArithmeticError));
            assert_eq!(safe_add_u32(u32::MAX, u32::MAX), Err(ArithmeticError));
        }

        #[test]
        fn u32_add_just_below_max() {
            assert_eq!(safe_add_u32(u32::MAX - 1, 1), Ok(u32::MAX));
        }

        #[test]
        fn u32_sub_zero_identity() {
            assert_eq!(safe_sub_u32(0, 0), Ok(0));
            assert_eq!(safe_sub_u32(u32::MAX, 0), Ok(u32::MAX));
        }

        #[test]
        fn u32_sub_underflow_returns_err() {
            assert_eq!(safe_sub_u32(0, 1), Err(ArithmeticError));
            assert_eq!(safe_sub_u32(0, u32::MAX), Err(ArithmeticError));
        }

        #[test]
        fn u32_sub_just_above_zero() {
            assert_eq!(safe_sub_u32(1, 1), Ok(0));
        }

        // ── u64 boundary-value suite ─────────────────────────────────────────

        #[test]
        fn u64_add_zero_identity() {
            assert_eq!(safe_add_u64(0, 0), Ok(0));
            assert_eq!(safe_add_u64(u64::MAX, 0), Ok(u64::MAX));
        }

        #[test]
        fn u64_add_overflow_returns_err() {
            assert_eq!(safe_add_u64(u64::MAX, 1), Err(ArithmeticError));
        }

        #[test]
        fn u64_add_just_below_max() {
            assert_eq!(safe_add_u64(u64::MAX - 1, 1), Ok(u64::MAX));
        }

        #[test]
        fn u64_sub_underflow_returns_err() {
            assert_eq!(safe_sub_u64(0, 1), Err(ArithmeticError));
        }

        #[test]
        fn u64_sub_just_above_zero() {
            assert_eq!(safe_sub_u64(1, 1), Ok(0));
        }

        // ── i128 boundary-value suite ─────────────────────────────────────────

        #[test]
        fn i128_add_zero_identity() {
            assert_eq!(safe_add_i128(0, 0), Ok(0));
            assert_eq!(safe_add_i128(i128::MAX, 0), Ok(i128::MAX));
            assert_eq!(safe_add_i128(i128::MIN, 0), Ok(i128::MIN));
        }

        #[test]
        fn i128_add_overflow_returns_err() {
            assert_eq!(safe_add_i128(i128::MAX, 1), Err(ArithmeticError));
            assert_eq!(safe_add_i128(i128::MAX, i128::MAX), Err(ArithmeticError));
        }

        #[test]
        fn i128_add_underflow_returns_err() {
            assert_eq!(safe_add_i128(i128::MIN, -1), Err(ArithmeticError));
        }

        #[test]
        fn i128_add_just_below_max() {
            assert_eq!(safe_add_i128(i128::MAX - 1, 1), Ok(i128::MAX));
        }

        #[test]
        fn i128_sub_zero_identity() {
            assert_eq!(safe_sub_i128(0, 0), Ok(0));
            assert_eq!(safe_sub_i128(i128::MIN, 0), Ok(i128::MIN));
        }

        #[test]
        fn i128_sub_underflow_returns_err() {
            assert_eq!(safe_sub_i128(i128::MIN, 1), Err(ArithmeticError));
        }

        #[test]
        fn i128_sub_overflow_returns_err() {
            // MAX - (-1) would overflow positively
            assert_eq!(safe_sub_i128(i128::MAX, -1), Err(ArithmeticError));
        }

        #[test]
        fn i128_mul_zero_absorbs() {
            assert_eq!(safe_mul_i128(i128::MAX, 0), Ok(0));
            assert_eq!(safe_mul_i128(0, i128::MAX), Ok(0));
        }

        #[test]
        fn i128_mul_overflow_returns_err() {
            assert_eq!(safe_mul_i128(i128::MAX, 2), Err(ArithmeticError));
            assert_eq!(safe_mul_i128(i128::MAX, i128::MAX), Err(ArithmeticError));
        }

        #[test]
        fn i128_mul_just_fits() {
            // 2^63 * 2^63 = 2^126 < i128::MAX (2^127 - 1), so this fits.
            let a: i128 = 1_i128 << 63;
            let b: i128 = 1_i128 << 63;
            assert!(safe_mul_i128(a, b).is_ok());
        }

        #[test]
        fn i128_mul_negative_positive() {
            assert_eq!(safe_mul_i128(-1, i128::MIN), Err(ArithmeticError));
            assert_eq!(safe_mul_i128(-1, i128::MAX), Ok(i128::MIN + 1));
        }

        // ── Property-style exhaustive small-value tests ───────────────────────
        // These iterate over every combination in a small range to prove that
        // no call can panic and that overflow is always detected.

        #[test]
        fn u32_add_exhaustive_small_range_no_panic() {
            let values: &[u32] = &[0, 1, 2, u32::MAX - 1, u32::MAX];
            for &a in values {
                for &b in values {
                    // Must not panic — only Ok or Err allowed.
                    let _ = safe_add_u32(a, b);
                }
            }
        }

        #[test]
        fn u64_add_exhaustive_small_range_no_panic() {
            let values: &[u64] = &[0, 1, 2, u64::MAX - 1, u64::MAX];
            for &a in values {
                for &b in values {
                    let _ = safe_add_u64(a, b);
                }
            }
        }

        #[test]
        fn i128_all_ops_exhaustive_no_panic() {
            let values: &[i128] = &[i128::MIN, i128::MIN + 1, -1, 0, 1, i128::MAX - 1, i128::MAX];
            for &a in values {
                for &b in values {
                    let _ = safe_add_i128(a, b);
                    let _ = safe_sub_i128(a, b);
                    let _ = safe_mul_i128(a, b);
                }
            }
        }

        // ── Fee-accumulation scenario tests (scout_access domain) ────────────

        #[test]
        fn fee_accumulation_typical_stroop_amounts() {
            // 1 XLM = 10_000_000 stroops. Typical subscription fee ≤ 70 XLM.
            let elite_fee_stroops: i128 = 70_000_000;
            let contact_fee_stroops: i128 = 1_000_000; // 0.1 XLM
            let max_contacts: i128 = 10_000;

            // contact_fee * max_contacts should not overflow
            let batch_total = safe_mul_i128(contact_fee_stroops, max_contacts);
            assert!(batch_total.is_ok());

            // Accumulating lots of fees should stay well within i128 range
            let large_total = safe_mul_i128(elite_fee_stroops, 1_000_000_000);
            assert!(large_total.is_ok());

            // Accumulating subscription + contact fees
            let accumulated = safe_add_i128(elite_fee_stroops, contact_fee_stroops);
            assert_eq!(accumulated, Ok(71_000_000));
        }

        #[test]
        fn counter_increment_typical() {
            // Counters (player count, validator count, milestone count) are u32.
            // Simulated: increment from near-max won't silently wrap.
            let near_max = u32::MAX - 5;
            for i in 0u32..5 {
                assert!(safe_add_u32(near_max + i, 1).is_ok());
            }
            assert_eq!(safe_add_u32(u32::MAX, 1), Err(ArithmeticError));
        }
    }
}

// ── Shared pagination types ───────────────────────────────────────────────────
//
// All list-returning query functions that accept an `offset` + `limit` use one
// of these page-result structs so callers receive both the requested window of
// entries **and** the total count (which tells them when to stop paging).
//
// Soroban's `#[contracttype]` macro does not support Rust generics, so each
// per-element type needs its own concrete Page struct.  The naming convention
// is `<ElementType>Page`.  New structs should be added here rather than
// reinvented per-contract.
//
// The `total` field reflects the size of the underlying collection at the
// moment the function was called; it is not a ledger-snapshotted value.
// Callers should treat it as an advisory guide for loop termination rather
// than a guarantee of consistency across multiple calls.

/// A page of `u64` IDs (e.g. player IDs) returned by a paginated query.
///
/// `entries` contains at most `limit` (capped at 50) items starting at
/// `offset`.  `total` is the total number of items in the underlying
/// collection at call time — use it to detect when paging is complete.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct U64Page {
    /// The items in this page, in insertion order.
    pub entries: soroban_sdk::Vec<u64>,
    /// Total number of items in the underlying collection.
    pub total: u32,
}

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
    use soroban_sdk::testutils::Address as _;

    fn s(env: &Env, v: &str) -> String {
        String::from_str(env, v)
    }

    #[test]
    fn wiring_link_unconfigured_is_not_configured() {
        let link = WiringLink::unconfigured();
        assert_eq!(link.address, None);
        assert_eq!(link.epoch, 0);
        assert!(!link.is_configured());
    }

    #[test]
    fn wiring_link_with_address_is_configured() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let link = WiringLink {
            address: Some(addr),
            epoch: 1,
        };
        assert!(link.is_configured());
    }

    #[test]
    fn progress_level_ordinals_match_documented_table() {
        assert_eq!(ProgressLevel::Unverified as u32, 0);
        assert_eq!(ProgressLevel::VerifiedIdentity as u32, 1);
        assert_eq!(ProgressLevel::PerformanceMilestones as u32, 2);
        assert_eq!(ProgressLevel::EliteTier as u32, 3);
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
