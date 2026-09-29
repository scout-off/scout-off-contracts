use scoutchain_shared_types::AdminError;
use soroban_sdk::contracterror;

/// Append-only: do not renumber existing variants. See docs/CONTRIBUTING.md.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ProgressError {
    // ── Initialization & lifecycle ──
    /// Contract has already been initialized and cannot be initialized again.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet; call `initialize` first.
    NotInitialized = 2,
    /// Contract is paused; all state-changing operations are blocked.
    ContractPaused = 3,

    // ── Authorization ──
    /// Caller is not authorized to perform this operation.
    Unauthorized = 4,

    // ── Business logic ──
    /// The requested level transition is not valid (e.g. skipping a level or going backwards).
    InvalidProgressTransition = 5,
    /// Player is already at the maximum level (EliteTier) and cannot advance further.
    AlreadyAtMaxLevel = 6,
    /// No progress record exists for the given player ID.
    PlayerNotFound = 7,

    // ── Cross-contract & arithmetic ──
    /// History counter overflowed the maximum u32 value.
    Overflow = 8,
    /// Call to registration contract failed.
    RegistrationCallFailed = 9,

    // ── Admin transfer ──
    /// `accept_admin` called before an admin transfer was proposed.
    PendingAdminNotSet = 10,

    // ── Migration ──
    /// Migration window is not currently active on this contract.
    /// Call `open_migration_window` (admin-only) before seeding state.
    MigrationNotActive = 11,
    /// A `HistoryEntry` already exists at `(player_id, history_index)` with
    /// different content. Identical replays are no-ops; conflicting replays
    /// are rejected to prevent silent overwriting of committed history.
    HistoryAlreadyExists = 12,
    /// The Merkle root independently recomputed from the seeded history does
    /// not match the `expected_root` supplied by the caller.
    /// The transaction is atomically rolled back — no partial state escapes.
    MerkleRootMismatch = 13,
    /// The supplied `history_index` is either zero, non-contiguous (gap in
    /// sequence), or would overwrite an existing entry at a different position.
    InvalidHistoryIndex = 14,
    /// `restore_player_level_record` targeted a player-level entry whose
    /// archival grace period has fully elapsed (evicted, not merely archived)
    /// and is unrecoverable.
    PlayerLevelRecordEvicted = 15,
}

impl AdminError for ProgressError {
    fn not_initialized() -> Self {
        ProgressError::NotInitialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_error_discriminants_remain_stable() {
        assert_eq!(ProgressError::AlreadyInitialized as u32, 1);
        assert_eq!(ProgressError::NotInitialized as u32, 2);
        assert_eq!(ProgressError::ContractPaused as u32, 3);
        assert_eq!(ProgressError::Unauthorized as u32, 4);
        assert_eq!(ProgressError::InvalidProgressTransition as u32, 5);
        assert_eq!(ProgressError::AlreadyAtMaxLevel as u32, 6);
        assert_eq!(ProgressError::PlayerNotFound as u32, 7);
        assert_eq!(ProgressError::Overflow as u32, 8);
        assert_eq!(ProgressError::RegistrationCallFailed as u32, 9);
        assert_eq!(ProgressError::PendingAdminNotSet as u32, 10);
    }
}
