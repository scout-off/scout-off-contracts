use soroban_sdk::{contracttype, Address, BytesN};

pub use scoutchain_shared_types::{ProgressLevel, WiringLink};

/// One step of a Merkle inclusion proof for [`ProgressEntry`] history
/// commitments (see [`DataKey::HistoryRoot`]).
///
/// `sibling` is the hash this step combines with the accumulated hash so
/// far; `sibling_is_right` records which side of the combination it sits
/// on (`H(current, sibling)` vs `H(sibling, current)`), since the RFC
/// 6962-style tree used here is not always evenly balanced and the
/// combination order is therefore not inferable from position alone.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryProofStep {
    pub sibling: BytesN<32>,
    pub sibling_is_right: bool,
}

/// A single entry in the immutable progress history
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProgressEntry {
    /// Unique player identifier whose level changed.
    pub player_id: u64,
    /// Player level before this history entry was recorded.
    pub old_level: ProgressLevel,
    /// Player level after this history entry was recorded.
    pub new_level: ProgressLevel,
    /// Wallet that triggered the update (validator or scout)
    pub updated_by: Address,
    /// Ledger timestamp when the level change was recorded, in Unix seconds.
    pub updated_at: u64,
    /// Milestone index from the verification contract that triggered this
    pub milestone_ref: u32,
    /// Ledger sequence number at the time of the level change
    pub ledger_sequence: u32,
}

/// One peak of a player's incremental Merkle frontier (issue #1368).
///
/// RFC 6962's Merkle Tree Hash decomposes a range of `n` leaves into a
/// canonical sequence of *perfect* subtrees whose sizes are the powers of two
/// present in the binary expansion of `n`. `FrontierPeak` records the root of
/// one such perfect subtree, together with the subtree's height `level` (a
/// subtree at level `k` spans `2^k` leaves).
///
/// The frontier for a player is the `Vec<FrontierPeak>` stored under
/// [`DataKey::HistoryFrontier`]. Because the number of peaks is exactly the
/// popcount of the leaf count, the frontier is bounded by 32 entries for any
/// history a 32-bit index can address, and appending a leaf costs O(log n)
/// hashes instead of O(n).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontierPeak {
    /// Height of this perfect subtree: it spans `2^level` leaves. Level 0 is a
    /// bare leaf hash.
    pub level: u32,
    /// Root hash of the perfect subtree (RFC 6962 `MTH` of those `2^level`
    /// consecutive leaves).
    pub hash: BytesN<32>,
}

/// Snapshot of all cross-contract peer addresses held by the progress
/// contract. Returned by [`ProgressContract::get_wiring_state`].
///
/// Each field is a [`WiringLink`] (from `scoutchain_shared_types`), matching
/// the pattern used by `registration`, `verification`, and `scout_access`.
/// This replaces the previous flat representation (issue #1412) where each
/// peer was split into separate `*_contract: Option<Address>` and `*_epoch:
/// u32` fields. The shared type makes generic tooling (scripts, off-chain
/// indexers) work uniformly across all four contracts without special-casing
/// progress.
///
/// See `docs/WIRING_REGISTRY_DESIGN.md` for the full design rationale and
/// the recommended migration path.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressWiringState {
    /// Link to the registration contract, set via `set_registration_contract`.
    /// Required for `advance_level` to validate player existence.
    pub registration_contract: WiringLink,
    /// Link to the verification contract, set via `set_verification_contract`.
    /// Only this address may call `advance_level` (primary authorised caller).
    pub verification_contract: WiringLink,
    /// Link to the scout_access contract, set via `set_scout_access_contract`.
    /// Whitelisted as the secondary authorised caller of `advance_level` for
    /// trial-offer Level-3 advances.
    pub scout_access_contract: WiringLink,
}

impl ProgressWiringState {
    /// Returns `true` iff all three peer address slots are populated.
    /// A return value of `false` means `advance_level` may fail because at
    /// least one expected caller or dependency address is missing.
    pub fn is_fully_wired(&self) -> bool {
        self.registration_contract.is_configured()
            && self.verification_contract.is_configured()
            && self.scout_access_contract.is_configured()
    }
}

#[contracttype]
pub enum DataKey {
    /// The `Address` of the contract administrator. Set during `initialize` and
    /// updated by `accept_admin`. Required for all privileged operations.
    Admin,
    /// Proposed replacement admin. The address stored here must call
    /// `accept_admin` before `Admin` is updated.
    PendingAdmin,
    /// Boolean flag (`true`) written during `initialize`. Absence or `false`
    /// means the contract has not yet been set up; `health()` reads this key.
    Initialized,
    /// Boolean flag indicating whether the contract is currently paused.
    /// `true` blocks all state-changing operations; `false` allows them.
    /// Toggled by `pause_contract` / `unpause_contract`.
    Paused,
    /// Maps a `player_id` (`u64`) to the player's current [`ProgressLevel`].
    /// Absent until the player's first level advancement; defaults to
    /// [`ProgressLevel::Unverified`] when read.
    PlayerLevel(u64),
    /// Tracks the total number of history entries recorded for a given
    /// `player_id`. Acts as a monotonically increasing counter; the current
    /// value is also the index of the most-recent [`HistoryEntry`].
    HistoryCounter(u64),
    /// Stores a [`ProgressEntry`] for a specific `(player_id, history_index)`
    /// pair. Indices start at `1` and are assigned by [`HistoryCounter`].
    HistoryEntry(u64, u32),
    /// Legacy unbounded snapshot of a player's entire history. This key is kept
    /// for compatibility with older deployments and recovery tooling, but new
    /// writes use bounded `HistoryPage(player_id, page)` shards instead so a
    /// single key no longer grows without a hard cap.
    HistoryVec(u64),
    /// Bounded page of player history entries. A player page stores at most
    /// `HISTORY_PAGE_SIZE` chronological entries, keeping each persistent-read key
    /// bounded even if a player accumulates many resets or re-entries.
    HistoryPage(u64, u32),
    /// The `Address` of the companion verification contract. Reserved for
    /// future cross-contract authorisation checks; not yet written at runtime.
    VerificationContract,
    /// The `Address` of the registration contract. Only this address is
    /// permitted to call `initialize_player`. Set by `set_registration_contract`.
    RegistrationContract,
    /// The `Address` of the scout_access contract. Whitelisted as a secondary
    /// authorised caller of `advance_level` (for trial-offer Level-3 advances).
    ScoutAccessContract,
    /// The storage layout version this contract is currently running. Absent
    /// means version 0 — the pre-versioning layout — so a contract that has
    /// never been migrated reads as "behind the code" rather than "current".
    SchemaVersion,
    /// How far a resumable migration has progressed. Stores the highest
    /// `player_id` the cursor has already visited, so a second `migrate` call
    /// resumes instead of re-scanning from zero. Absent means "not started".
    MigrationCursor(u64),
    /// Total items rewritten by a migration across all calls. Diagnostic only:
    /// it lets an operator confirm progress across several `migrate` calls
    /// without reading the cursor's implied position.
    MigrationProcessed,
    /// The current Merkle commitment root over a player's full
    /// [`ProgressEntry`] history (an RFC 6962-style Merkle Tree Hash — see
    /// `record_progress_entry`'s doc comment for the construction). Updated
    /// on every history append alongside [`HistoryVec`]. Independently
    /// verifiable via `verify_history_proof` without trusting the RPC node
    /// that served the query — see `get_progress_root`.
    HistoryRoot(u64),
    /// Incremental Merkle commitment state for a player's history (issue
    /// #1368): the ordered list of [`FrontierPeak`]s whose levels are the set
    /// bits of the current entry count.
    ///
    /// This is the accumulator `record_progress_entry` now maintains instead
    /// of re-reading every `HistoryPage` shard and re-hashing the whole
    /// history on each append. The RFC 6962 root in [`DataKey::HistoryRoot`]
    /// remains byte-identical — it is now derived by folding these peaks
    /// instead of by recursing over the full leaf list.
    ///
    /// Absent for players whose history predates this key; the first append
    /// after an upgrade rebuilds it lazily from the existing history.
    HistoryFrontier(u64),

    /// Boolean flag (`true`) written by `open_migration_window`; absent or
    /// `false` means the migration window is closed. All `admin_seed_*`
    /// functions on this contract check this flag before writing any state.
    /// Cleared by `close_migration_window`. Stored in instance storage so it
    /// is immediately visible and requires no TTL management.
    MigrationActive,
    /// Re-wiring epoch for [`DataKey::RegistrationContract`], bumped by
    /// every `set_registration_contract` call. See
    /// `scoutchain_shared_types::WiringLink` and
    /// `docs/WIRING_REGISTRY_DESIGN.md` (issue #1041).
    RegistrationContractEpoch,
    /// Re-wiring epoch for [`DataKey::VerificationContract`], bumped by
    /// every `set_verification_contract` call.
    VerificationContractEpoch,
    /// Re-wiring epoch for [`DataKey::ScoutAccessContract`], bumped by
    /// every `set_scout_access_contract` call.
    ScoutAccessContractEpoch,
}
