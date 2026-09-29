use soroban_sdk::{contracttype, Address, BytesN};

pub use scoutchain_shared_types::ProgressLevel;

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

/// Snapshot of all cross-contract peer addresses held by the progress
/// contract. Returned by [`ProgressContract::get_wiring_state`].
///
/// Use this to verify — without inspecting storage keys directly — that all
/// three peer links are configured. See `docs/WIRING_REGISTRY_DESIGN.md` for
/// the full design rationale and the recommended migration path.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressWiringState {
    /// Address of the registration contract, if set via
    /// `set_registration_contract`. Required for `advance_level` to validate
    /// player existence via the registration contract.
    pub registration_contract: Option<Address>,
    /// Address of the verification contract, if set via
    /// `set_verification_contract`. Only this address may call `advance_level`
    /// (primary authorised caller).
    pub verification_contract: Option<Address>,
    /// Address of the scout_access contract, if set via
    /// `set_scout_access_contract`. Whitelisted as the secondary authorised
    /// caller of `advance_level` for trial-offer Level-3 advances.
    pub scout_access_contract: Option<Address>,
    /// Re-wiring epoch for `registration_contract` — bumped on every
    /// `set_registration_contract` call. `0` iff `registration_contract` is
    /// `None`. Added additively (issue #1041); see
    /// `scoutchain_shared_types::WiringLink` for what epoch is for.
    pub registration_epoch: u32,
    /// Re-wiring epoch for `verification_contract`.
    pub verification_epoch: u32,
    /// Re-wiring epoch for `scout_access_contract`.
    pub scout_access_epoch: u32,
}

impl ProgressWiringState {
    /// Returns `true` iff all three peer address slots are populated.
    /// A return value of `false` means `advance_level` may fail because at
    /// least one expected caller or dependency address is missing.
    pub fn is_fully_wired(&self) -> bool {
        self.registration_contract.is_some()
            && self.verification_contract.is_some()
            && self.scout_access_contract.is_some()
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
    /// The current Merkle commitment root over a player's full
    /// [`ProgressEntry`] history (an RFC 6962-style Merkle Tree Hash — see
    /// `record_progress_entry`'s doc comment for the construction). Updated
    /// on every history append alongside [`HistoryVec`]. Independently
    /// verifiable via `verify_history_proof` without trusting the RPC node
    /// that served the query — see `get_progress_root`.
    HistoryRoot(u64),

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
