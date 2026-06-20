use soroban_sdk::{contracttype, Address};

pub use scoutchain_shared_types::{ContractHealth, ProgressLevel};

/// A single entry in the immutable progress history
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProgressEntry {
    pub player_id: u64,
    pub old_level: ProgressLevel,
    pub new_level: ProgressLevel,
    /// Wallet that triggered the update (validator or scout)
    pub updated_by: Address,
    pub updated_at: u64,
    /// Milestone index from the verification contract that triggered this
    pub milestone_ref: u32,
    /// Ledger sequence number at the time of the level change
    pub ledger_sequence: u32,
}

#[contracttype]
pub enum DataKey {
    /// The `Address` of the contract administrator. Set during `initialize` and
    /// updated by `transfer_admin`. Required for all privileged operations.
    Admin,
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
    /// The `Address` of the companion verification contract. Reserved for
    /// future cross-contract authorisation checks; not yet written at runtime.
    VerificationContract,
    /// The `Address` of the registration contract. Only this address is
    /// permitted to call `initialize_player`. Set by `set_registration_contract`.
    RegistrationContract,
}
